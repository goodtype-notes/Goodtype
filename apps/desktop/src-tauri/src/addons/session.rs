use super::manifest::API;
use serde::{Deserialize, de};
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 4 * 1024 * 1024;
/// Room for the JSON-RPC envelope around a captured scope.
pub(super) const MAX_SCOPE: usize = MAX_FRAME - 1024;
const STDERR_LIMIT: usize = 16 * 1024;
const OFFER: [&str; 3] = ["progress", "cancel", "page-batch-v0"];
const POLL: Duration = Duration::from_millis(20);
/// The math worker accepts 65,536 request IDs per process; retire it before the last one.
const MAX_REQUESTS: u64 = 65_536;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Protocol,
    NeedsUpdate,
    Timeout,
    Cancelled,
    Unavailable,
    /// The restricted launcher refused or failed; never retried without confinement.
    Sandbox,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandError {
    pub code: i32,
    pub(super) message: String,
    pub data: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Initialize {
    api_version: String,
    capabilities: Vec<String>,
    addon: Identity,
}

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    id: String,
    phase: String,
    completed: u64,
    total: u64,
}

enum Message {
    Reply(Result<Value, CommandError>),
    Progress,
}

struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Strict;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
                let mut object = Map::new();
                while let Some((key, Strict(value))) = map.next_entry()? {
                    if object.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(Strict(Value::Object(object)))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
                let mut list = Vec::new();
                while let Some(Strict(value)) = seq.next_element()? {
                    list.push(value);
                }
                Ok(Strict(Value::Array(list)))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Strict, E> {
                Ok(Strict(value.into()))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

fn read_frame(reader: &mut impl BufRead) -> Result<Value, Failure> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_FRAME + 2) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| Failure::Protocol)?;
    let frame = bytes.strip_suffix(b"\n").ok_or(Failure::Protocol)?;
    let frame = frame.strip_suffix(b"\r").unwrap_or(frame);
    if frame.len() > MAX_FRAME {
        return Err(Failure::Protocol);
    }
    let Strict(value) = serde_json::from_slice(frame).map_err(|_| Failure::Protocol)?;
    Ok(value)
}

fn encode(value: &Value) -> Result<Vec<u8>, Failure> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| Failure::Protocol)?;
    if bytes.len() > MAX_FRAME {
        return Err(Failure::Protocol);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

fn message(value: Value, expected: &str, progress: bool) -> Result<Message, Failure> {
    let Value::Object(mut fields) = value else {
        return Err(Failure::Protocol);
    };
    if fields
        .remove("jsonrpc")
        .and_then(|v| v.as_str().map(str::to_owned))
        .as_deref()
        != Some("2.0")
        || fields.len() != 2
    {
        return Err(Failure::Protocol);
    }
    if let Some(id) = fields.remove("id") {
        if id.as_str() != Some(expected) {
            return Err(Failure::Protocol);
        }
        if let Some(result) = fields.remove("result") {
            return Ok(Message::Reply(Ok(result)));
        }
        let error: CommandError =
            serde_json::from_value(fields.remove("error").ok_or(Failure::Protocol)?)
                .map_err(|_| Failure::Protocol)?;
        if error.message.chars().count() > 512 {
            return Err(Failure::Protocol);
        }
        return Ok(Message::Reply(Err(error)));
    }
    if !progress || fields.remove("method").as_ref() != Some(&json!("command/progress")) {
        return Err(Failure::Protocol);
    }
    let tick: Progress = serde_json::from_value(fields.remove("params").ok_or(Failure::Protocol)?)
        .map_err(|_| Failure::Protocol)?;
    if tick.id != expected
        || tick.phase != "recognizing"
        || tick.total == 0
        || tick.completed > tick.total
    {
        return Err(Failure::Protocol);
    }
    Ok(Message::Progress)
}

fn capabilities(reply: Value, id: &str, version: &str) -> Result<HashSet<String>, Failure> {
    let reply: Initialize = serde_json::from_value(reply).map_err(|_| Failure::NeedsUpdate)?;
    let count = reply.capabilities.len();
    let capabilities: HashSet<_> = reply.capabilities.into_iter().collect();
    if reply.api_version != API
        || reply.addon.id != id
        || reply.addon.version != version
        || capabilities.len() != count
        || !capabilities.contains("page-batch-v0")
        || capabilities
            .iter()
            .any(|capability| !OFFER.contains(&capability.as_str()))
    {
        return Err(Failure::NeedsUpdate);
    }
    Ok(capabilities)
}

type WriteRequest = (Vec<u8>, mpsc::SyncSender<bool>);

pub(super) struct Session {
    process: Option<Launched>,
    input: mpsc::SyncSender<WriteRequest>,
    output: mpsc::Receiver<Result<Value, Failure>>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
    capabilities: HashSet<String>,
    next_id: u64,
    pub used: Instant,
}

/// A `stdio-jsonrpc` addon in a fresh AppContainer with only `registryRead` (no network
/// capability), read-only access to its verified install, and one private low-integrity scratch
/// directory. `registryRead` was approved by the user on 2026-11-21: without it the math runtime
/// blocks before loading ONNX Runtime; the Python runner holds the same capability.
/// A launch or ACL failure makes the addon unavailable; there is no unconfined fallback.
pub(super) struct Launched {
    // Declared first so the job is shut down before the scratch directory is removed.
    child: crate::sandbox::SandboxedChild,
    _scratch: tempfile::TempDir,
}

fn launch(install: &Path, executable: &Path, scratch_root: &Path) -> Result<Launched, Failure> {
    if !super::platform_supported() {
        return Err(Failure::Unavailable);
    }
    // The launcher rejects reparse points in both trees (and their ancestors) before any ACL.
    let scratch = tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(scratch_root)
        .map_err(|_| Failure::Sandbox)?;
    let env = crate::sandbox::minimal_environment().map_err(|_| Failure::Sandbox)?;
    let child = crate::sandbox::launch(crate::sandbox::SandboxSpec {
        executable,
        args: &["--session".into()],
        cwd: scratch.path(),
        env,
        read_only: &[install],
        scratch: Some(scratch.path()),
        capabilities: &["registryRead"],
        memory_limit: crate::recognition::math_process::MEMORY_BYTES,
        active_process_limit: 1,
        stdio: true,
    })
    .map_err(|error| {
        eprintln!("Addon sandbox launch failed: {error}");
        Failure::Sandbox
    })?;
    Ok(Launched {
        child,
        _scratch: scratch,
    })
}

impl Session {
    /// `executable` must be the exact file inside `install` that was just verified.
    pub(super) fn start(
        install: &Path,
        executable: &Path,
        scratch_root: &Path,
        id: &str,
        version: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, Failure> {
        let mut process = launch(install, executable, scratch_root)?;
        let stdin = process.child.stdin.take().ok_or(Failure::Unavailable)?;
        let stdout = process.child.stdout.take().ok_or(Failure::Unavailable)?;
        let stderr = process.child.stderr.take().ok_or(Failure::Unavailable)?;
        Self::connect(stdin, stdout, stderr, Some(process), id, version, cancelled)
    }

    fn connect(
        mut stdin: impl Write + Send + 'static,
        stdout: impl Read + Send + 'static,
        mut stderr: impl Read + Send + 'static,
        process: Option<Launched>,
        id: &str,
        version: &str,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self, Failure> {
        let (input, writes) = mpsc::sync_channel::<WriteRequest>(1);
        std::thread::spawn(move || {
            while let Ok((bytes, ack)) = writes.recv() {
                let ok = stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_ok();
                let _ = ack.send(ok);
                if !ok {
                    break;
                }
            }
        });
        let (frames, output) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let frame = read_frame(&mut stdout);
                let failed = frame.is_err();
                if frames.send(frame).is_err() || failed {
                    break;
                }
            }
        });
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let drain = Arc::clone(&diagnostics);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(count @ 1..) = stderr.read(&mut chunk) {
                let mut retained = drain.lock().unwrap();
                let keep = count.min(STDERR_LIMIT - retained.len());
                retained.extend_from_slice(&chunk[..keep]);
            }
        });
        let mut session = Self {
            process,
            input,
            output,
            diagnostics,
            capabilities: HashSet::new(),
            next_id: 0,
            used: Instant::now(),
        };
        let reply = session
            .call(
                "initialize",
                json!({
                    "apiVersion": API, "capabilities": OFFER,
                    "addon": {"id": id, "version": version}
                }),
                Duration::from_secs(60),
                cancelled,
            )?
            .map_err(|_| Failure::NeedsUpdate)?;
        session.capabilities = capabilities(reply, id, version)?;
        Ok(session)
    }

    pub(super) fn healthy(&mut self) -> bool {
        // Any output between requests is unsolicited, including a repeated terminal reply.
        matches!(self.output.try_recv(), Err(mpsc::TryRecvError::Empty))
            && self.running()
            && self.next_id < MAX_REQUESTS
            && self.used.elapsed() < Duration::from_secs(600)
    }

    fn running(&mut self) -> bool {
        self.process
            .as_mut()
            .is_none_or(|process| process.child.running())
    }

    /// Bounded stderr, for explicit diagnostics only; never shown as status or persisted.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn diagnostics(&self) -> Vec<u8> {
        self.diagnostics.lock().unwrap().clone()
    }

    pub(super) fn invoke(
        &mut self,
        params: Value,
        cancelled: impl Fn() -> bool,
    ) -> Result<Result<Value, CommandError>, Failure> {
        self.call("command/invoke", params, Duration::from_secs(30), cancelled)
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancelled: impl Fn() -> bool,
    ) -> Result<Result<Value, CommandError>, Failure> {
        let deadline = Instant::now() + timeout;
        self.next_id = self.next_id.checked_add(1).ok_or(Failure::Protocol)?;
        let id = self.next_id.to_string();
        let bytes = encode(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))?;
        if cancelled() {
            return Err(Failure::Cancelled);
        }
        let (ack, written) = mpsc::sync_channel(1);
        self.input
            .try_send((bytes, ack))
            .map_err(|_| Failure::Protocol)?;
        loop {
            match written.recv_timeout(self.remaining(deadline, &id, &cancelled)?) {
                Ok(true) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                _ => return Err(Failure::Protocol),
            }
        }
        loop {
            let wait = self.remaining(deadline, &id, &cancelled)?;
            match self.output.recv_timeout(wait) {
                Ok(frame) => match message(frame?, &id, self.capabilities.contains("progress"))? {
                    Message::Reply(reply) => {
                        self.remaining(deadline, &id, &cancelled)?;
                        self.used = Instant::now();
                        return Ok(reply);
                    }
                    Message::Progress => {}
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Failure::Protocol),
            }
        }
    }

    fn remaining(
        &mut self,
        deadline: Instant,
        id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Duration, Failure> {
        if cancelled() {
            if self.capabilities.contains("cancel") {
                let cancel =
                    json!({"jsonrpc":"2.0", "method":"command/cancel", "params":{"id":id}});
                let (ack, _) = mpsc::sync_channel(1);
                let _ = self.input.try_send((encode(&cancel)?, ack));
                let grace = Instant::now() + Duration::from_secs(1);
                while Instant::now() < grace && self.running() {
                    let _ = self.output.recv_timeout(POLL);
                }
            }
            return Err(Failure::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Failure::Timeout);
        }
        Ok(remaining.min(POLL))
    }
}

type Warm = (String, String, Session);

pub(crate) struct Runtime(Arc<Mutex<Option<Warm>>>);

impl Default for Runtime {
    fn default() -> Self {
        let slot: Arc<Mutex<Option<Warm>>> = Arc::new(Mutex::new(None));
        let weak = Arc::downgrade(&slot);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let Some(slot) = weak.upgrade() else { break };
                if let Ok(mut slot) = slot.try_lock()
                    && slot
                        .as_mut()
                        .is_some_and(|(_, _, worker)| !worker.healthy())
                {
                    slot.take();
                }
            }
        });
        Self(slot)
    }
}

impl Runtime {
    pub(super) fn shutdown(&self) {
        self.0.lock().unwrap().take();
    }

    pub(super) fn run(
        &self,
        app: &tauri::AppHandle,
        addon: &'static super::Addon,
        command: &str,
        scope: Value,
        context: Option<&str>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Result<Value, CommandError>, Failure> {
        use tauri::Manager;

        let state = app.state::<super::Importers>();
        let (job, _) = state.get(addon.id).map_err(|_| Failure::Unavailable)?;
        let mut slot = self.0.try_lock().map_err(|_| Failure::Unavailable)?;
        // Polled every 20 ms, so in memory only: lifecycle commands reserve the job before
        // changing the active release or policy. Disk is checked here, once, and at commit.
        let cancelled = || cancelled() || job.busy();
        if !super::manager::enabled(app, addon).unwrap_or(false) {
            return Err(Failure::Unavailable);
        }
        let (version, path) = super::install_dir(app, addon)
            .map_err(|_| Failure::Unavailable)?
            .active()
            .ok_or(Failure::Unavailable)?;
        if job.state.lock().unwrap().incompatible_session.as_deref() == Some(&version) {
            return Err(Failure::NeedsUpdate);
        }
        if cancelled() {
            return Err(Failure::Cancelled);
        }
        let manifest = super::manager::installed(addon, &path, &version, true).map_err(
            |problem| match problem {
                super::manifest::Error::Incompatible(_) => Failure::NeedsUpdate,
                _ => Failure::Unavailable,
            },
        )?;
        if !manifest.supports_selection(command)
            || !addon
                .permissions()
                .iter()
                .all(|p| manifest.permissions.contains(p))
        {
            return Err(Failure::Unavailable);
        }
        let fingerprint = format!(
            "{}:{}:{}",
            addon.id,
            version,
            super::installation_digest(&path).map_err(|_| Failure::Unavailable)?
        );
        let mut worker = match slot.take() {
            Some((key, previous, worker))
                if key == fingerprint && Some(previous.as_str()) == context =>
            {
                Some(worker)
            }
            _ => None,
        };
        if worker.as_mut().is_some_and(|worker| !worker.healthy()) {
            worker.take();
        }
        let mut worker = match worker {
            Some(worker) => worker,
            None => match Session::start(
                &path,
                &path.join(addon.executable()),
                &scratch_root(app)?,
                addon.id,
                &version,
                cancelled,
            ) {
                Err(Failure::NeedsUpdate) => {
                    job.state.lock().unwrap().incompatible_session = Some(version);
                    return Err(Failure::NeedsUpdate);
                }
                result => result?,
            },
        };
        // Manifest v1 currently admits only empty settings descriptors.
        let reply = worker.invoke(
            json!({"command":command,"scope":scope,"settings":{}}),
            cancelled,
        )?;
        if let Some(context) = context {
            *slot = Some((fingerprint, context.into(), worker));
        }
        Ok(reply)
    }
}

/// Per-process scratch lives under app-local data, beside (not inside) the install directories.
fn scratch_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, Failure> {
    use tauri::Manager;
    let root = app
        .path()
        .app_local_data_dir()
        .map_err(|_| Failure::Sandbox)?
        .join("addon-sessions");
    std::fs::create_dir_all(&root).map_err(|_| Failure::Sandbox)?;
    Ok(root)
}

impl Failure {
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::NeedsUpdate => "addon-needs-update",
            Self::Cancelled => "addon-cancelled",
            Self::Timeout => "addon-timeout",
            Self::Unavailable => "addon-unavailable",
            Self::Sandbox => "addon-sandbox-failed",
            Self::Protocol => "addon-invalid-response",
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
