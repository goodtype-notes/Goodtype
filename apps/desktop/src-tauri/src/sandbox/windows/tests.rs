use super::*;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn handle_count() -> u32 {
    let mut count = 0;
    check(
        unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
        "GetProcessHandleCount",
    )
    .unwrap();
    count
}

fn profile_directory(child: &SandboxedChild) -> PathBuf {
    use windows_sys::Win32::Security::Isolation::GetAppContainerFolderPath;
    let mut path = null_mut();
    let sid = wide(&child.process._profile.sid_text);
    let result = unsafe { GetAppContainerFolderPath(sid.as_ptr(), &mut path) };
    assert_eq!(result, 0);
    unsafe {
        use std::os::windows::ffi::OsStringExt;
        let len = (0..).take_while(|index| *path.add(*index) != 0).count();
        let directory = OsString::from_wide(std::slice::from_raw_parts(path, len)).into();
        CoTaskMemFree(path.cast());
        directory
    }
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(memory: *const c_void);
}

#[test]
#[ignore = "native Windows AppContainer test; needs rustc and a Windows user profile"]
fn native_appcontainer_probe() {
    let base = tempfile::tempdir_in(std::env::var_os("USERPROFILE").unwrap()).unwrap();
    let stage = base.path().join("runtime with spaces");
    let scratch = base.path().join("scratch");
    fs::create_dir(&stage).unwrap();
    fs::create_dir(&scratch).unwrap();
    let exe = stage.join("sandbox_probe.exe");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sandbox/windows/probe.rs");
    assert!(
        Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "--crate-name", "sandbox_probe"])
            .arg(source)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap()
            .success()
    );
    let allowed = stage.join("allowed.txt");
    fs::write(&allowed, "allowed").unwrap();
    let denied = base.path().join("private.txt");
    fs::write(&denied, "private").unwrap();
    let sentinel = fs::File::open(&denied).unwrap();
    // An inheritable handle outside the standard stream allowlist must be excluded.
    check(
        unsafe {
            SetHandleInformation(
                sentinel.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        },
        "SetHandleInformation",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let quote = r#"arg with "quote" and trailing slash\"#;
    let args = [
        allowed.into_os_string(),
        denied.into_os_string(),
        stage.join("forbidden.txt").into_os_string(),
        listener.local_addr().unwrap().to_string().into(),
        (sentinel.as_raw_handle() as usize).to_string().into(),
        quote.into(),
        scratch.as_os_str().to_owned(),
    ];
    let stage_root = [stage.as_path()];
    let launch_with = |args: &[OsString], capabilities: &[&str], profile: Option<&str>| {
        super::launch(SandboxSpec {
            executable: &exe,
            args,
            cwd: &scratch,
            env: super::super::minimal_environment().unwrap(),
            read_only: if profile.is_some() { &[] } else { &stage_root },
            scratch: Some(&scratch),
            capabilities,
            profile,
            memory_limit: 128 * 1024 * 1024,
            active_process_limit: 1,
            stdio: true,
        })
    };
    let launch_probe = |args: &[OsString]| launch_with(args, &[], None).unwrap();

    // Control: the same probe outside the sandbox reaches the listener.
    let control = Command::new(&exe)
        .args(["net", &listener.local_addr().unwrap().to_string()])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&control.stdout).trim(), "connected");
    listener.accept().unwrap();
    println!("control (unsandboxed) loopback TCP: connected");

    let run = |mut child: SandboxedChild| {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"echo probe")
            .unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        let mut errors = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut errors)
            .unwrap();
        let status = child.wait().unwrap();
        assert!(
            status.success(),
            "{status:?}
{output}
{errors}"
        );
        assert_eq!(
            errors,
            "stderr probe
"
        );
        (child, output)
    };
    // Zero capabilities: Winsock does not start (WSAEPROVIDERFAILEDINIT, 10107). With
    // registryRead it starts and socket() is denied (WSAEACCES, 10013).
    for (capabilities, network) in [
        (&[][..], "denied: WSAStartup 10107"),
        (&["registryRead"][..], "denied: socket 10013"),
    ] {
        let child = launch_with(&args, capabilities, None).unwrap();
        let profile = profile_directory(&child);
        assert!(profile.is_dir(), "missing live profile: {profile:?}");
        let (child, output) = run(child);
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 9, "{output}");
        assert_eq!(
            [&lines[..4], &lines[6..]].concat(),
            [
                "allowed read: allowed",
                "private read: 5",
                "outside write: 5",
                "scratch write: ok",
                quote,
                "echo probe",
                "ambient handle: absent",
            ]
        );
        assert_eq!(
            lines[4],
            format!("loopback TCP: {network}"),
            "{capabilities:?}"
        );
        assert_eq!(
            lines[5],
            format!("external TCP: {network}"),
            "{capabilities:?}"
        );
        println!(
            "capabilities {capabilities:?}:
{output}"
        );
        drop(child);
        assert!(!profile.exists(), "profile still exists: {profile:?}");
    }
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "the sandboxed child reached the host listener"
    );

    // A stable profile keeps its tree grant across launches until it is revoked.
    let name = format!("Goodtype.Addon.probe.{}", std::process::id());
    let granting = Instant::now();
    grant_profile(&name, &stage).unwrap();
    println!("stable grant: {:?}", granting.elapsed());
    for launch in 1..=2 {
        let started = Instant::now();
        let child = launch_with(&args, &["registryRead"], Some(&name)).unwrap();
        let spawned = started.elapsed();
        let profile = profile_directory(&child);
        let (child, output) = run(child);
        assert!(
            output.starts_with(
                "allowed read: allowed
"
            ),
            "{output}"
        );
        drop(child);
        assert!(
            profile.is_dir(),
            "stable profile removed by launch {launch}"
        );
        println!("stable launch {launch}: spawned in {spawned:?}, profile kept");
    }
    revoke_profile(&name, Some(&stage)).unwrap();
    // Without the grant the child cannot read its own tree (or is not created at all).
    if let Ok(child) = launch_with(&args, &[], Some(&name)) {
        assert!(!child.wait().unwrap().success());
    }
    revoke_profile(&name, None).unwrap();
    println!("stable profile revoked");

    // A per-launch profile of a process that no longer exists is swept; a live one is kept.
    let dead = Profile::open("Goodtype.Sandbox.4000000000.1.1", &[], true).unwrap();
    let live = Profile::new(&[]).unwrap();
    let folder = |profile: &Profile| {
        Path::new(&std::env::var_os("LOCALAPPDATA").unwrap())
            .join("Packages")
            .join(String::from_utf16_lossy(&profile.name[..profile.name.len() - 1]).to_lowercase())
    };
    let (dead_folder, live_folder) = (folder(&dead), folder(&live));
    assert!(dead_folder.is_dir() && live_folder.is_dir());
    std::mem::forget(dead);
    sweep_transient_profiles();
    assert!(!dead_folder.exists(), "stale profile kept: {dead_folder:?}");
    assert!(live_folder.is_dir(), "live profile swept: {live_folder:?}");
    drop(live);
    println!("stale per-launch profiles swept, live ones kept");

    // The first launch loads one-time OS state; later launches must not grow the host.
    let settle = || {
        let (mut last, mut stable) = (handle_count(), 0);
        let deadline = Instant::now() + Duration::from_secs(20);
        while stable < 10 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            let now = handle_count();
            stable = if now == last { stable + 1 } else { 0 };
            last = now;
        }
        last
    };
    let baseline = settle();
    for iteration in 1..=5 {
        let child = launch_probe(&["idle".into()]);
        let profile = profile_directory(&child);
        assert!(profile.is_dir(), "missing live profile: {profile:?}");
        assert!(child.running());
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(124));
        drop(child);
        assert!(!profile.exists(), "profile still exists: {profile:?}");
        println!("launch/kill {iteration}: exit 124, profile removed");
    }

    let after = settle();
    assert!(
        after <= baseline,
        "handle count grew from {baseline} to {after}"
    );
    println!("host handles: {baseline} after the first launch, {after} after 5 more launches");
}
