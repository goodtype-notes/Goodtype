use super::{SandboxSpec, SandboxedChild};
use std::ffi::{OsStr, OsString, c_void};
use std::mem::{size_of, size_of_val, zeroed};
use std::os::windows::{
    ffi::OsStrExt,
    fs::MetadataExt,
    io::{AsRawHandle, OwnedHandle},
    process::CommandExt,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};
use std::{fs, io};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    DeriveCapabilitySidsFromName, FreeSid, PSID, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
};
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Threading::*;

#[cfg(test)]
mod tests;

/// How long a kill waits for the job to empty before it closes the job instead.
const STOP: Duration = Duration::from_secs(5);
/// Per-launch profile name prefixes. Stable addon profiles are swept by their owner.
const TRANSIENT: [&str; 2] = ["goodtype.sandbox.", "goodtype.calculation."];

/// Every icacls run in the process is serialized: a recursive revoke of one profile must never
/// interleave with a recursive grant on the same tree.
static ACL: Mutex<()> = Mutex::new(());

fn icacls(path: &Path, args: &[&OsStr]) -> Result<(), String> {
    let tool = super::system_root()?.join("System32/icacls.exe");
    let _serial = ACL.lock().unwrap_or_else(PoisonError::into_inner);
    let mut command = Command::new(tool);
    command
        .creation_flags(CREATE_NO_WINDOW)
        .arg(path)
        .args(args);
    let output = super::output(&mut command).map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "icacls {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn grant_acl(path: &Path, sid: &str, permission: &str) -> Result<(), String> {
    let inheritance = if path.is_dir() { "(OI)(CI)" } else { "" };
    let ace = OsString::from(format!("*{sid}:{inheritance}{permission}"));
    icacls(
        path,
        &[
            "/grant".as_ref(),
            &ace,
            "/T".as_ref(),
            "/Q".as_ref(),
            "/L".as_ref(),
        ],
    )
}

fn revoke_acl(path: &Path, sid: &str) -> Result<(), String> {
    let ace = OsString::from(format!("*{sid}"));
    icacls(
        path,
        &[
            "/remove:g".as_ref(),
            &ace,
            "/T".as_ref(),
            "/Q".as_ref(),
            "/L".as_ref(),
        ],
    )
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn check(result: i32, operation: &str) -> Result<(), String> {
    if result == 0 {
        Err(format!("{operation}: {}", io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

struct Handle(HANDLE);
// Windows kernel handles support concurrent waits, queries and termination.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn sid_text(sid: PSID) -> Result<String, String> {
    let mut text = null_mut();
    check(
        unsafe { ConvertSidToStringSidW(sid, &mut text) },
        "ConvertSidToStringSidW",
    )?;
    unsafe {
        let length = (0..).take_while(|index| *text.add(*index) != 0).count();
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        Ok(value)
    }
}

fn delete_profile(name: &[u16]) {
    let result = unsafe { DeleteAppContainerProfile(name.as_ptr()) };
    if result < 0 {
        eprintln!("Sandbox profile cleanup failed: {result:#x}");
    }
}

struct Profile {
    name: Vec<u16>,
    sid: PSID,
    sid_text: String,
    capabilities: Vec<SID_AND_ATTRIBUTES>,
    grants: Vec<PathBuf>,
    /// A per-launch profile is deleted, and its grants revoked, when the launch ends. A stable
    /// profile belongs to its caller, which grants and revokes its tree outside any launch.
    transient: bool,
}

impl Profile {
    fn new(capability_names: &[&str]) -> Result<Self, String> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let name = format!(
            "Goodtype.Sandbox.{}.{}.{}",
            std::process::id(),
            timestamp,
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        Self::open(&name, capability_names, true)
    }

    fn open(name: &str, capability_names: &[&str], transient: bool) -> Result<Self, String> {
        let name = wide(name);
        let mut sid = null_mut();
        let mut result = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        // HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS): a stable profile from an earlier grant.
        if !transient && result == (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as i32 {
            result = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        }
        if result < 0 {
            return Err(format!("AppContainer profile HRESULT {result:#x}"));
        }
        let mut profile = Self {
            name,
            sid,
            sid_text: String::new(),
            capabilities: Vec::new(),
            grants: Vec::new(),
            transient,
        };
        profile.sid_text = sid_text(sid)?;
        for name in capability_names {
            let mut groups = null_mut();
            let mut group_count = 0;
            let mut capabilities = null_mut();
            let mut count = 0;
            check(
                unsafe {
                    DeriveCapabilitySidsFromName(
                        wide(name).as_ptr(),
                        &mut groups,
                        &mut group_count,
                        &mut capabilities,
                        &mut count,
                    )
                },
                "DeriveCapabilitySidsFromName",
            )?;
            unsafe {
                for index in 0..group_count {
                    LocalFree((*groups.add(index as usize)).cast());
                }
                LocalFree(groups.cast());
                for index in 0..count {
                    profile.capabilities.push(SID_AND_ATTRIBUTES {
                        Sid: *capabilities.add(index as usize),
                        Attributes: 4,
                    });
                }
                LocalFree(capabilities.cast());
            }
        }
        Ok(profile)
    }

    fn grant(&mut self, path: &Path, permission: &str) -> Result<(), String> {
        self.grants.push(path.to_path_buf());
        grant_acl(path, &self.sid_text, permission)
    }
}

// The owned SID allocations remain immutable after process creation.
unsafe impl Send for Profile {}
unsafe impl Sync for Profile {}

impl Drop for Profile {
    fn drop(&mut self) {
        // Revoking walks every file of the runtime (thousands with NumPy, SciPy and Matplotlib) and
        // took ~15 s, during which ending a session or changing Python appeared to hang. The
        // job has already stopped. Delete the profile synchronously and revoke its ACEs in
        // the background; the ACL lock keeps that revoke from interleaving with a later grant.
        // A stable profile's scratch grant dies with the scratch directory.
        let grants = std::mem::take(&mut self.grants);
        if self.transient {
            let sid = std::mem::take(&mut self.sid_text);
            if !grants.is_empty() {
                std::thread::spawn(move || {
                    for path in grants {
                        if inspect_tree(&path).is_ok()
                            && let Err(error) = revoke_acl(&path, &sid)
                        {
                            eprintln!("Sandbox ACL revoke failed: {error}");
                        }
                    }
                });
            }
            delete_profile(&self.name);
        }
        unsafe {
            FreeSid(self.sid);
            for capability in &self.capabilities {
                LocalFree(capability.Sid.cast());
            }
        }
    }
}

/// Creates (or reopens) the stable profile `name` and grants it read and execute on `root`, once.
/// Launches that name the profile then grant only their scratch.
pub(super) fn grant_profile(name: &str, root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("Sandbox ACL path must be absolute".into());
    }
    inspect_tree(root)?;
    let profile = Profile::open(name, &[], false)?;
    grant_acl(root, &profile.sid_text, "RX")
}

/// Revokes a stable profile's grant on `root` (when it still exists) and deletes the profile.
pub(super) fn revoke_profile(name: &str, root: Option<&Path>) -> Result<(), String> {
    let wide_name = wide(name);
    let mut sid = null_mut();
    let result = unsafe { DeriveAppContainerSidFromAppContainerName(wide_name.as_ptr(), &mut sid) };
    if result < 0 {
        return Err(format!(
            "DeriveAppContainerSidFromAppContainerName {result:#x}"
        ));
    }
    let text = sid_text(sid);
    unsafe { FreeSid(sid) };
    let text = text?;
    if let Some(root) = root.filter(|root| root.exists()) {
        inspect_tree(root)?;
        revoke_acl(root, &text)?;
    }
    delete_profile(&wide_name);
    Ok(())
}

fn process_alive(pid: u32) -> bool {
    let process = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
    if process.0.is_null() {
        // A process of another user or integrity level is alive but not openable.
        return io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
    }
    let mut code = 0;
    unsafe { GetExitCodeProcess(process.0, &mut code) != 0 && code == STILL_ACTIVE as u32 }
}

/// Deletes per-launch profiles left by a crashed Goodtype. Profiles of a live process (another
/// window, a test run) are kept. Their ACEs sat on per-run trees that are already gone.
pub(super) fn sweep_transient_profiles() {
    let Some(packages) =
        std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("Packages"))
    else {
        return;
    };
    let Ok(entries) = fs::read_dir(packages) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some(rest) = TRANSIENT
            .iter()
            .find_map(|prefix| name.strip_prefix(prefix))
        else {
            continue;
        };
        let Some(pid) = rest
            .split('.')
            .next()
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() && !process_alive(pid) {
            delete_profile(&wide(&name));
        }
    }
}

struct Attributes {
    // usize storage keeps the opaque attribute list properly aligned.
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl Attributes {
    fn new(count: u32) -> Result<Self, String> {
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut bytes) };
        if bytes == 0 {
            return Err(format!(
                "InitializeProcThreadAttributeList: {}",
                io::Error::last_os_error()
            ));
        }
        let mut storage = vec![0; bytes.div_ceil(size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        check(
            unsafe { InitializeProcThreadAttributeList(pointer, count, 0, &mut bytes) },
            "InitializeProcThreadAttributeList",
        )?;
        Ok(Self {
            _storage: storage,
            pointer,
        })
    }

    fn set<T>(&mut self, key: u32, value: &T) -> Result<(), String> {
        check(
            unsafe {
                UpdateProcThreadAttribute(
                    self.pointer,
                    0,
                    key as usize,
                    (value as *const T).cast::<c_void>(),
                    size_of::<T>(),
                    null_mut(),
                    null(),
                )
            },
            "UpdateProcThreadAttribute",
        )
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
    }
}

fn plain(path: &Path) -> Result<fs::Metadata, String> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!(
            "Sandbox path contains a reparse point: {}",
            path.display()
        ));
    }
    Ok(metadata)
}

fn inspect_tree(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors().skip(1) {
        plain(ancestor)?;
    }
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        if plain(&path)?.is_dir() {
            for entry in fs::read_dir(&path).map_err(|error| error.to_string())? {
                pending.push(entry.map_err(|error| error.to_string())?.path());
            }
        }
    }
    Ok(())
}

// The Windows C runtime escapes quotes and trailing backslashes with doubled backslashes.
fn append_arg(command: &mut Vec<u16>, arg: &OsStr) -> Result<(), String> {
    const SLASH: u16 = b'\\' as u16;
    const QUOTE: u16 = b'"' as u16;
    command.push(QUOTE);
    let mut slashes = 0;
    for unit in arg.encode_wide() {
        match unit {
            0 => return Err("Sandbox argument contains a NUL".into()),
            SLASH => slashes += 1,
            _ => {
                command.extend(std::iter::repeat_n(
                    SLASH,
                    if unit == QUOTE {
                        slashes * 2 + 1
                    } else {
                        slashes
                    },
                ));
                command.push(unit);
                slashes = 0;
            }
        }
    }
    command.extend(std::iter::repeat_n(SLASH, slashes * 2));
    command.push(QUOTE);
    Ok(())
}

/// Scratch is the one writable root, so it must not overlap a read-only root or hold the
/// executable. A batch file would run through cmd.exe with its own argument parsing.
fn check_paths(spec: &SandboxSpec<'_>) -> Result<(), String> {
    let extension = spec
        .executable
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase());
    if matches!(extension.as_deref(), Some("bat" | "cmd")) {
        return Err("Sandbox executable must not be a batch file".into());
    }
    let Some(scratch) = spec.scratch else {
        return Ok(());
    };
    let canonical = |path: &Path| fs::canonicalize(path).map_err(|error| error.to_string());
    let scratch = canonical(scratch)?;
    if canonical(spec.executable)?.starts_with(&scratch) {
        return Err("Sandbox executable must not be inside the scratch directory".into());
    }
    for root in spec.read_only {
        let root = canonical(root)?;
        if root.starts_with(&scratch) || scratch.starts_with(&root) {
            return Err("Sandbox scratch must not overlap a read-only root".into());
        }
    }
    Ok(())
}

pub(super) struct Process {
    process: Handle,
    job: Handle,
    _profile: Profile,
}

pub(super) fn launch(mut spec: SandboxSpec<'_>) -> Result<SandboxedChild, String> {
    if !spec.executable.is_absolute()
        || !spec.cwd.is_absolute()
        || spec.memory_limit == 0
        || spec.active_process_limit == 0
    {
        return Err("Sandbox requires absolute executable and working directory paths and nonzero job limits".into());
    }
    check_paths(&spec)?;
    let mut command = Vec::new();
    append_arg(&mut command, spec.executable.as_os_str())?;
    for arg in spec.args {
        command.push(b' ' as u16);
        append_arg(&mut command, arg)?;
    }
    command.push(0);
    let executable = wide(spec.executable);
    let cwd = wide(spec.cwd);
    if cwd[..cwd.len() - 1].contains(&0) {
        return Err("Sandbox working directory contains a NUL".into());
    }
    if let Some(scratch) = spec.scratch {
        // Scratch owns TEMP and TMP whatever the caller's spelling of the names.
        spec.env.retain(|key, _| {
            !["TEMP", "TMP"]
                .iter()
                .any(|name| key.eq_ignore_ascii_case(name))
        });
        for key in ["TEMP", "TMP"] {
            spec.env.insert(key.into(), scratch.as_os_str().to_owned());
        }
    }
    let mut environment = Vec::new();
    let mut entries = std::collections::BTreeMap::new();
    for (key, value) in spec.env {
        let key = key
            .to_str()
            .filter(|key| !key.is_empty() && !key.contains(['=', '\0']))
            .ok_or("Invalid sandbox environment variable name")?;
        let value = wide(value);
        if value[..value.len() - 1].contains(&0)
            || entries.insert(key.to_uppercase(), value).is_some()
        {
            return Err("Invalid or duplicate sandbox environment variable".into());
        }
    }
    for (key, value) in entries {
        environment.extend(key.encode_utf16());
        environment.push(b'=' as u16);
        environment.extend(value);
    }
    if environment.is_empty() {
        environment.push(0);
    }
    environment.push(0);
    for path in spec.read_only.iter().copied().chain(spec.scratch) {
        if !path.is_absolute() {
            return Err(format!(
                "Sandbox ACL path must be absolute: {}",
                path.display()
            ));
        }
        inspect_tree(path)?;
    }
    let mut profile = match spec.profile {
        Some(name) => Profile::open(name, spec.capabilities, false)?,
        None => Profile::new(spec.capabilities)?,
    };
    for path in spec.read_only {
        profile.grant(path, "RX")?;
    }
    if let Some(scratch) = spec.scratch {
        profile.grant(scratch, "M")?;
        icacls(
            scratch,
            &[
                "/setintegritylevel".as_ref(),
                "(OI)(CI)L".as_ref(),
                "/T".as_ref(),
                "/Q".as_ref(),
                "/L".as_ref(),
            ],
        )
        .map_err(|error| format!("Could not set low integrity: {error}"))?;
    }
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid,
        Capabilities: if profile.capabilities.is_empty() {
            null_mut()
        } else {
            profile.capabilities.as_mut_ptr()
        },
        CapabilityCount: profile.capabilities.len() as u32,
        Reserved: 0,
    };
    // The one-process Job Object denies descendants without the incompatible creation mitigation.
    let mut attributes = Attributes::new(if spec.stdio { 3 } else { 2 })?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    let restricted_packages = 1u32;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
        &restricted_packages,
    )?;
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESHOWWINDOW;
    startup.StartupInfo.wShowWindow = 0; // SW_HIDE: the runner must not open a visible console.
    startup.lpAttributeList = attributes.pointer;
    let job = Handle(unsafe { CreateJobObjectW(null(), null()) });
    if job.0.is_null() {
        return Err(io::Error::last_os_error().to_string());
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
        | if spec.active_process_limit > 1 {
            JOB_OBJECT_LIMIT_JOB_MEMORY
        } else {
            0
        };
    limits.BasicLimitInformation.ActiveProcessLimit = spec.active_process_limit;
    limits.ProcessMemoryLimit = spec.memory_limit;
    limits.JobMemoryLimit = spec.memory_limit;
    check(
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        },
        "SetInformationJobObject",
    )?;
    let (mut child_pipes, mut host_pipes) = (None, None);
    if spec.stdio {
        let (child_in, host_in) = io::pipe().map_err(|error| error.to_string())?;
        let (host_out, child_out) = io::pipe().map_err(|error| error.to_string())?;
        let (host_err, child_err) = io::pipe().map_err(|error| error.to_string())?;
        child_pipes = Some([
            OwnedHandle::from(child_in),
            OwnedHandle::from(child_out),
            OwnedHandle::from(child_err),
        ]);
        host_pipes = Some((host_in, host_out, host_err));
    }
    let child_handles = child_pipes
        .as_ref()
        .map(|pipes| pipes.each_ref().map(AsRawHandle::as_raw_handle));
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    {
        // From the first inheritable child end until those ends are closed, no other spawn in
        // this crate may run: a Command spawn inherits every inheritable handle in the process.
        let _spawn = super::spawn_lock();
        let created = (|| {
            if let Some(handles) = &child_handles {
                for handle in handles {
                    check(
                        unsafe {
                            SetHandleInformation(*handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
                        },
                        "SetHandleInformation",
                    )?;
                }
                startup.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
                [
                    startup.StartupInfo.hStdInput,
                    startup.StartupInfo.hStdOutput,
                    startup.StartupInfo.hStdError,
                ] = *handles;
                attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles)?;
            }
            check(
                unsafe {
                    CreateProcessW(
                        executable.as_ptr(),
                        command.as_mut_ptr(),
                        null(),
                        null(),
                        i32::from(spec.stdio),
                        CREATE_SUSPENDED
                            | CREATE_NO_WINDOW
                            | CREATE_UNICODE_ENVIRONMENT
                            | EXTENDED_STARTUPINFO_PRESENT,
                        environment.as_ptr().cast(),
                        cwd.as_ptr(),
                        &startup.StartupInfo,
                        &mut info,
                    )
                },
                "CreateProcessW",
            )
        })();
        drop(child_pipes);
        created?;
    }
    drop(attributes);
    let process = Handle(info.hProcess);
    let thread = Handle(info.hThread);
    if let Err(error) = check(
        unsafe { AssignProcessToJobObject(job.0, process.0) },
        "AssignProcessToJobObject",
    ) {
        unsafe {
            TerminateProcess(process.0, 1);
            WaitForSingleObject(process.0, STOP.as_millis() as u32);
        }
        return Err(error);
    }
    let process = Process {
        process,
        job,
        _profile: profile,
    };
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        return Err(format!("ResumeThread: {}", io::Error::last_os_error()));
    }
    let (stdin, stdout, stderr) = match host_pipes {
        Some((stdin, stdout, stderr)) => (Some(stdin), Some(stdout), Some(stderr)),
        None => (None, None, None),
    };
    Ok(SandboxedChild {
        stdin,
        stdout,
        stderr,
        process,
    })
}

impl Process {
    /// Terminates the job and waits up to [`STOP`] for it to empty.
    pub(super) fn kill(&self) -> Result<(), String> {
        let deadline = Instant::now() + STOP;
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        loop {
            check(
                unsafe {
                    QueryInformationJobObject(
                        self.job.0,
                        JobObjectBasicAccountingInformation,
                        (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                        size_of_val(&accounting) as u32,
                        null_mut(),
                    )
                },
                "QueryInformationJobObject",
            )?;
            if accounting.ActiveProcesses == 0 {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Sandbox job did not stop in time".into());
            }
            check(
                unsafe { TerminateJobObject(self.job.0, 124) },
                "TerminateJobObject",
            )?;
            unsafe { WaitForSingleObject(self.process.0, remaining.as_millis() as u32) };
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn wait(&self) -> Result<std::process::ExitStatus, String> {
        if unsafe { WaitForSingleObject(self.process.0, INFINITE) } != WAIT_OBJECT_0 {
            return Err(format!(
                "WaitForSingleObject: {}",
                io::Error::last_os_error()
            ));
        }
        let mut code = 0;
        check(
            unsafe { GetExitCodeProcess(self.process.0, &mut code) },
            "GetExitCodeProcess",
        )?;
        use std::os::windows::process::ExitStatusExt;
        Ok(std::process::ExitStatus::from_raw(code))
    }

    pub(super) fn running(&self) -> bool {
        unsafe { WaitForSingleObject(self.process.0, 0) == WAIT_TIMEOUT }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Err(error) = self.kill() {
            eprintln!("Sandbox job shutdown failed: {error}");
            // Close the kill-on-close job, then wait a bounded time before dropping the profile.
            drop(std::mem::replace(&mut self.job, Handle(null_mut())));
            unsafe { WaitForSingleObject(self.process.0, STOP.as_millis() as u32) };
        }
    }
}
