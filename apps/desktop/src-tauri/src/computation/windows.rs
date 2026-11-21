use crate::sandbox::{self, SandboxSpec, SandboxedChild};
use std::{ffi::OsString, path::Path};

pub struct Process(SandboxedChild);

impl Process {
    pub fn start(runtime: &Path, input: &Path, scratch: &Path) -> Result<Self, String> {
        let mut env = sandbox::minimal_environment()?;
        for key in [
            "ALLUSERSPROFILE",
            "APPDATA",
            "COMSPEC",
            "PROGRAMDATA",
            "SYSTEMDRIVE",
            "WINDIR",
        ] {
            if let Ok(value) = std::env::var(key) {
                env.insert(key.into(), value.into());
            }
        }
        for key in ["MPLCONFIGDIR", "IPYTHONDIR"] {
            env.insert(key.into(), scratch.as_os_str().to_owned());
        }
        for (key, value) in [
            ("OPENBLAS_NUM_THREADS", "1"),
            ("PATH", ""),
            ("PYTHONHASHSEED", "0"),
            ("PYTHONUTF8", "1"),
        ] {
            env.insert(key.into(), value.into());
        }
        let mut args = ["-I", "-S", "-B", "-X", "utf8"]
            .map(OsString::from)
            .to_vec();
        args.push(input.join("runner.py").into_os_string());
        args.push(input.as_os_str().to_owned());
        sandbox::launch(SandboxSpec {
            executable: &runtime.join("python.exe"),
            args: &args,
            cwd: scratch,
            env,
            read_only: &[runtime, input],
            scratch: Some(scratch),
            capabilities: &["registryRead"],
            profile: None,
            memory_limit: 1024 * 1024 * 1024,
            active_process_limit: 1,
            stdio: false,
        })
        .map(Self)
    }

    pub fn stop(&self) {
        let _ = self.0.kill();
    }

    pub fn running(&self) -> bool {
        self.0.running()
    }
}
