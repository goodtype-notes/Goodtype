//! Host-owned staging and restricted process launch.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{Mutex, MutexGuard, PoisonError},
};

#[cfg(windows)]
mod windows;

/// Held around every process creation in this crate. The launcher makes its three child pipe
/// ends inheritable until `CreateProcessW` returns; a `Command` spawn in that window would
/// inherit them, because std spawns with handle inheritance on. Spawns made by third-party
/// crates (Tauri, WebView2) or by test code that bypasses these helpers are not covered.
static SPAWN: Mutex<()> = Mutex::new(());

pub(crate) fn spawn_lock() -> MutexGuard<'static, ()> {
    SPAWN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `Command::spawn` under [`spawn_lock`]. Only the creation is serialized, not the child's life.
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    let _spawn = spawn_lock();
    command.spawn()
}

/// `Command::output` with the same defaults, spawning under [`spawn_lock`].
pub(crate) fn output(command: &mut Command) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn(command)?.wait_with_output()
}

/// All paths and arguments must come from the host. Keep staged trees private to
/// the host until launch and keep their storage alive until the child is dropped.
pub struct SandboxSpec<'a> {
    pub executable: &'a Path,
    /// Arguments after argv[0]. The launcher applies Windows command-line quoting.
    pub args: &'a [OsString],
    pub cwd: &'a Path,
    pub env: BTreeMap<OsString, OsString>,
    pub read_only: &'a [&'a Path],
    pub scratch: Option<&'a Path>,
    /// Use an empty slice for the default profile with no granted capabilities.
    pub capabilities: &'a [&'a str],
    /// `None` creates a fresh profile for this launch, deleted with it. `Some` names a stable
    /// profile prepared by [`grant_profile`], whose tree grant outlives the launch; the launch
    /// then grants only `read_only` and `scratch`, and leaves the profile in place.
    pub profile: Option<&'a str>,
    /// Committed memory per process (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`). With more than one
    /// active process the same value also caps the whole job.
    pub memory_limit: usize,
    pub active_process_limit: u32,
    pub stdio: bool,
}

pub struct SandboxedChild {
    pub stdin: Option<io::PipeWriter>,
    pub stdout: Option<io::PipeReader>,
    pub stderr: Option<io::PipeReader>,
    #[cfg(windows)]
    process: windows::Process,
    #[cfg(not(windows))]
    _unsupported: std::convert::Infallible,
}

impl SandboxedChild {
    /// Wait for the executable. Dropping the child also shuts down its entire job.
    pub fn wait(&self) -> Result<ExitStatus, String> {
        #[cfg(windows)]
        return self.process.wait();
        #[cfg(not(windows))]
        match self._unsupported {}
    }

    pub fn kill(&self) -> Result<(), String> {
        #[cfg(windows)]
        return self.process.kill();
        #[cfg(not(windows))]
        match self._unsupported {}
    }

    pub fn running(&self) -> bool {
        #[cfg(windows)]
        return self.process.running();
        #[cfg(not(windows))]
        match self._unsupported {}
    }
}

pub fn launch(spec: SandboxSpec<'_>) -> Result<SandboxedChild, String> {
    #[cfg(windows)]
    return windows::launch(spec);
    #[cfg(not(windows))]
    {
        let _ = spec;
        Err("Sandbox launch is unsupported on this platform".into())
    }
}

/// Grants the stable profile `name` read and execute on the verified tree `root`, once.
pub fn grant_profile(name: &str, root: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return windows::grant_profile(name, root);
    #[cfg(not(windows))]
    {
        let _ = (name, root);
        Err("Sandbox launch is unsupported on this platform".into())
    }
}

/// Revokes a stable profile's grant on `root`, when given and present, and deletes the profile.
pub fn revoke_profile(name: &str, root: Option<&Path>) -> Result<(), String> {
    #[cfg(windows)]
    return windows::revoke_profile(name, root);
    #[cfg(not(windows))]
    {
        let _ = (name, root);
        Ok(())
    }
}

/// Deletes per-launch profiles left behind by a Goodtype process that no longer runs.
pub fn sweep_transient_profiles() {
    #[cfg(windows)]
    windows::sweep_transient_profiles();
}

#[cfg(windows)]
fn system_root() -> Result<std::path::PathBuf, String> {
    // Ask Windows directly so an inherited PATH cannot select the ACL tool.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    use std::os::windows::ffi::OsStringExt;
    let mut path = [0; 260];
    let len = unsafe { GetWindowsDirectoryW(path.as_mut_ptr(), path.len() as u32) } as usize;
    if len == 0 || len >= path.len() {
        return Err("Could not resolve the Windows directory".into());
    }
    Ok(OsString::from_wide(&path[..len]).into())
}

/// Build the explicit OS environment needed for Windows profile resolution.
/// Launch sets TEMP and TMP in the supplied block when there is scratch.
/// Windows can replace them with the AppContainer profile's temporary directory.
#[cfg(windows)]
pub fn minimal_environment() -> Result<BTreeMap<OsString, OsString>, String> {
    let root = system_root()?;
    let mut environment = BTreeMap::from([("SYSTEMROOT".into(), root.into_os_string())]);
    for key in ["LOCALAPPDATA", "USERPROFILE"] {
        let value = std::env::var_os(key).ok_or_else(|| format!("{key} is missing"))?;
        environment.insert(key.into(), value);
    }
    Ok(environment)
}

#[cfg(not(windows))]
pub fn minimal_environment() -> Result<BTreeMap<OsString, OsString>, String> {
    Err("Sandbox launch is unsupported on this platform".into())
}
