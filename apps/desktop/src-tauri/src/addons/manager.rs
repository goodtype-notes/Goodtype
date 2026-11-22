use super::*;
use manifest::{API, Error, Manifest, Permission};
use std::collections::BTreeMap;

const CONFIG: &str = "addons.json";
const MAX_POLICY_BYTES: usize = 64 * 1024;
static POLICY: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Policy {
    allow_direct_apply: bool,
    #[serde(default = "auto_check_default")]
    auto_check: bool,
    addons: BTreeMap<String, Preference>,
    // A persisted record of consent for the current attempt. Never reused on retry or restart.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pending_install_approvals: BTreeMap<String, Vec<Permission>>,
}

fn auto_check_default() -> bool {
    true
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            allow_direct_apply: false,
            auto_check: auto_check_default(),
            addons: BTreeMap::new(),
            pending_install_approvals: BTreeMap::new(),
        }
    }
}

pub(super) fn auto_check(app: &tauri::AppHandle) -> bool {
    policy(app).is_ok_and(|policy| policy.auto_check)
}

#[tauri::command]
pub(crate) async fn addon_set_auto_check(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || patch(&app, |policy| policy.auto_check = enabled))
        .await
        .map_err(|error| error.to_string())?
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Preference {
    enabled: bool,
    direct_apply: bool,
}

fn read_policy_file(path: &Path) -> Result<Policy, String> {
    serde_json::from_slice(&read_bounded(path, MAX_POLICY_BYTES)?)
        .map_err(|error| error.to_string())
}

fn first_run(path: &Path) -> bool {
    matches!(path.try_exists(), Ok(false))
        && matches!(path.with_extension("json.bak").try_exists(), Ok(false))
}

fn read_policy(path: &Path) -> Result<Policy, String> {
    if first_run(path) {
        return Ok(Policy::default());
    }
    read_policy_file(path).or_else(|_| {
        let backup = read_policy_file(&path.with_extension("json.bak"))?;
        crate::store::write_json(path, &backup, MAX_POLICY_BYTES)?;
        Ok(backup)
    })
}

fn write_policy(path: &Path, policy: &Policy) -> Result<(), String> {
    let backup = path.with_extension("json.bak");
    let prior = read_policy_file(path).ok();
    let bytes = crate::store::json_bytes(policy, MAX_POLICY_BYTES)?;
    crate::store::write_atomic(&backup, &bytes)?;
    if let Err(error) = crate::store::write_atomic(path, &bytes) {
        if let Some(prior) = prior {
            crate::store::write_json(&backup, &prior, MAX_POLICY_BYTES)?;
        }
        return Err(error);
    }
    Ok(())
}

fn patch_file(path: &Path, update: impl FnOnce(&mut Policy)) -> Result<(), String> {
    let _lock = POLICY.lock().unwrap();
    let mut policy = read_policy(path)?;
    update(&mut policy);
    write_policy(path, &policy)
}

fn patch(app: &tauri::AppHandle, update: impl FnOnce(&mut Policy)) -> Result<(), String> {
    patch_file(&crate::settings::config_file(app, CONFIG)?, update)
}

fn policy(app: &tauri::AppHandle) -> Result<Policy, String> {
    let _lock = POLICY.lock().unwrap();
    read_policy(&crate::settings::config_file(app, CONFIG)?)
}

pub(super) fn manifest_problem(error: Error) -> String {
    match error {
        Error::NeedsReinstall => "addon needs reinstall".into(),
        Error::Invalid => "invalid addon manifest or package".into(),
        Error::Incompatible(_) => "incompatible addon API".into(),
    }
}

pub(super) fn installed(
    addon: &Addon,
    path: &Path,
    version: &str,
    verify: bool,
) -> Result<Manifest, Error> {
    if is_package_link(&fs::symlink_metadata(path).map_err(|_| Error::Invalid)?) {
        return Err(Error::Invalid);
    }
    // A same-user process that can rewrite app-local data is outside the threat model.
    let provenance: Provenance = read_bounded(&path.join(PROVENANCE), 4096)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or(Error::NeedsReinstall)?;
    if !update::is_hash(&provenance.archive_sha256, 64)
        || installation_digest(path).map_err(|_| Error::Invalid)? != provenance.inventory_sha256
    {
        return Err(Error::NeedsReinstall);
    }
    let legacy_pin = if !path.join(manifest::FILE).exists() && addon.legacy == Some(version) {
        Some(
            addon
                .pinned
                .filter(|pin| {
                    pin.version == version
                        && pin.sha256.eq_ignore_ascii_case(&provenance.archive_sha256)
                })
                .ok_or(Error::NeedsReinstall)?,
        )
    } else {
        None
    };
    let inventory = if verify {
        verify_package(addon, path).map_err(|_| Error::Invalid)?
    } else {
        serde_json::from_slice(
            &read_bounded(&path.join(PACKAGE_MANIFEST), MAX_JSON_BYTES)
                .map_err(|_| Error::Invalid)?,
        )
        .map_err(|_| Error::Invalid)?
    };
    if inventory.version != version {
        return Err(Error::Invalid);
    }
    check_contract(addon, &inventory, None, legacy_pin.as_ref()).map_err(|_| Error::Invalid)?;
    manifest::load(addon, path, version, &inventory, legacy_pin.as_ref())
}

pub(crate) fn startup(app: &tauri::AppHandle) -> Result<(), String> {
    if !platform_supported() {
        return Ok(());
    }
    let path = crate::settings::config_file(app, CONFIG)?;
    let migrate = first_run(&path);
    let mut defaults = Policy::default();
    let _ = policy(app);
    for addon in CATALOG {
        let dir = install_dir(app, addon)?;
        discard_staging(&dir)?;
        let Some((version, path)) = dir.active() else {
            continue;
        };
        match installed(addon, &path, &version, true) {
            Ok(_) if migrate && !path.join(manifest::FILE).exists() => {
                defaults.addons.insert(
                    addon.id.into(),
                    Preference {
                        enabled: true,
                        direct_apply: false,
                    },
                );
            }
            Err(Error::Invalid) => {
                app.state::<Importers>()
                    .get(addon.id)?
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .startup_invalid = true;
            }
            _ => {}
        }
    }
    if migrate {
        let _lock = POLICY.lock().unwrap();
        write_policy(&crate::settings::config_file(app, CONFIG)?, &defaults)?;
    }
    Ok(())
}

fn discard_staging(dir: &InstallDir) -> Result<(), String> {
    let root = fs::canonicalize(&dir.0).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(&root)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        let name = entry.file_name();
        let Some((pid, nonce)) = name
            .to_str()
            .and_then(|name| name.strip_prefix("staging-"))
            .and_then(|suffix| suffix.split_once('-'))
        else {
            continue;
        };
        if u32::from_str_radix(pid, 16).is_err() || u128::from_str_radix(nonce, 16).is_err() {
            continue;
        }
        if let Ok(metadata) = fs::symlink_metadata(entry.path())
            && metadata.is_dir()
            && !is_package_link(&metadata)
            && let Ok(path) = fs::canonicalize(entry.path())
            && path.parent() == Some(root.as_path())
        {
            fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn await_approval(job: &Importer, approval: Approval) -> Result<(), String> {
    let mut state = job.state.lock().unwrap();
    if job.cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    state.activity = Some(Activity::AwaitingApproval { approval });
    let state = job
        .import_finished
        .wait_while(state, |state| {
            !job.cancel.load(Ordering::SeqCst)
                && matches!(state.activity, Some(Activity::AwaitingApproval { .. }))
        })
        .unwrap();
    if !matches!(state.activity, Some(Activity::Installing)) {
        return Err(CANCELLED.into());
    }
    Ok(())
}

#[tauri::command]
pub fn addon_approve(
    importers: tauri::State<'_, Importers>,
    id: String,
    token: String,
) -> Result<(), String> {
    let (job, _) = importers.get(&id)?;
    let mut state = job.state.lock().unwrap();
    if job.cancel.load(Ordering::SeqCst)
        || !matches!(&state.activity, Some(Activity::AwaitingApproval { approval }) if approval.token == token)
    {
        return Err("approval-unavailable".into());
    }
    state.activity = Some(Activity::Installing);
    job.import_finished.notify_all();
    Ok(())
}

pub(super) fn record_install_approval(
    app: &tauri::AppHandle,
    id: &str,
    approved: &[Permission],
) -> Result<(), String> {
    patch(app, |policy| {
        policy
            .pending_install_approvals
            .insert(id.into(), approved.to_vec());
    })
}

pub(super) fn clear_install_approval(app: &tauri::AppHandle, id: &str) {
    if let Err(error) = patch(app, |policy| {
        policy.pending_install_approvals.remove(id);
    }) {
        eprintln!("Could not clear addon install approval: {error}");
    }
}

fn requires_approval(
    approved: Option<&[Permission]>,
    candidate: &Manifest,
    previous: Option<&Manifest>,
    recorded: bool,
) -> bool {
    match approved {
        Some(approved) => candidate.permissions.iter().any(|p| !approved.contains(p)),
        None => !recorded || !previous.is_some_and(|old| old.same_scope(candidate)),
    }
}

pub(super) fn install(
    app: &tauri::AppHandle,
    dir: &InstallDir,
    archive: &Path,
    release: &PinnedRelease,
    approved: Option<&[Permission]>,
    problem: &mut InstallProblem,
) -> Result<String, String> {
    let previous = dir.active();
    let previous_declaration = previous
        .as_ref()
        .and_then(|(version, path)| installed(dir.1, path, version, true).ok());
    let importers = app.state::<Importers>();
    let (job, _) = importers.get(dir.1.id)?;
    *problem = InstallProblem::Policy;
    let prior_policy = policy(app)?;
    if let Some(approved) = approved
        && prior_policy
            .pending_install_approvals
            .get(dir.1.id)
            .map(Vec::as_slice)
            != Some(approved)
    {
        return Err("install approval unavailable".into());
    }
    let recorded = prior_policy.addons.contains_key(dir.1.id);
    let phase = std::cell::Cell::new(InstallProblem::Verify);
    let result = install_with_policy(
        dir,
        archive,
        release,
        &|path| {
            phase.set(InstallProblem::Probe);
            probe_tool(dir.1, path)
        },
        |candidate| {
            phase.set(InstallProblem::Policy);
            if requires_approval(approved, candidate, previous_declaration.as_ref(), recorded) {
                await_approval(
                    job,
                    Approval {
                        token: unique_suffix(),
                        version: release.version.into(),
                        permissions: candidate.permissions.clone(),
                    },
                )?;
            } else if job.cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            shutdown(app, dir.1);
            Ok(())
        },
        |candidate| {
            if job.cancel.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            phase.set(InstallProblem::Policy);
            activate(
                app,
                dir,
                (previous.is_some(), previous_declaration.as_ref()),
                release.version,
                candidate,
                approved.is_some(),
            )
        },
    );
    *problem = phase.get();
    result
}

fn activate(
    app: &tauri::AppHandle,
    dir: &InstallDir,
    previous: (bool, Option<&Manifest>),
    version: &str,
    candidate: &Manifest,
    enable_after_install: bool,
) -> Result<(), String> {
    let _lock = POLICY.lock().unwrap();
    let path = crate::settings::config_file(app, CONFIG)?;
    let mut policy = read_policy(&path)?;
    let prior = policy.addons.get(dir.1.id).copied();
    let next = install_preference(
        prior,
        previous.0,
        previous.1,
        candidate,
        enable_after_install,
    );
    let pending = policy.pending_install_approvals.remove(dir.1.id);
    if Some(next) == prior && pending.is_none() {
        return dir.activate(version);
    }
    policy.addons.insert(dir.1.id.into(), next);
    write_policy(&path, &policy)?;
    if let Err(error) = dir.activate(version) {
        if let Some(pending) = pending {
            policy
                .pending_install_approvals
                .insert(dir.1.id.into(), pending);
        }
        if let Some(prior) = prior {
            policy.addons.insert(dir.1.id.into(), prior);
        } else {
            policy.addons.remove(dir.1.id);
        }
        write_policy(&path, &policy)?;
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub async fn addon_rollback(app: tauri::AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let importers = app.state::<Importers>();
        let (job, addon) = importers.get(&id)?;
        if !platform_supported() {
            return Err("unsupported-os".into());
        }
        job.begin(Activity::Installing)?;
        let result = (|| {
            let dir = install_dir(&app, addon)?;
            let (version, path) = dir.previous().ok_or("no previous addon release")?;
            let declaration = installed(addon, &path, &version, true).map_err(manifest_problem)?;
            shutdown(&app, addon);
            let active = dir.active();
            let active_declaration = active
                .as_ref()
                .and_then(|(version, path)| installed(addon, path, version, true).ok());
            activate(
                &app,
                &dir,
                (active.is_some(), active_declaration.as_ref()),
                &version,
                &declaration,
                false,
            )?;
            job.state.lock().unwrap().startup_invalid = false;
            Ok(())
        })();
        job.set(Activity::Idle);
        result
    })
    .await
    .map_err(|error| error.to_string())?
}

fn install_preference(
    prior: Option<Preference>,
    had_install: bool,
    previous: Option<&Manifest>,
    candidate: &Manifest,
    enable_after_install: bool,
) -> Preference {
    let mut preference = if previous.is_some_and(|old| old.same_scope(candidate)) {
        prior.unwrap_or_default()
    } else {
        Preference::default()
    };
    if !had_install || enable_after_install {
        preference.enabled = true;
    }
    preference
}

pub(super) fn direct_apply(app: &tauri::AppHandle, id: &str) -> bool {
    policy(app).is_ok_and(|policy| {
        policy.allow_direct_apply
            && policy
                .addons
                .get(id)
                .is_some_and(|pref| pref.enabled && pref.direct_apply)
    })
}

pub(super) fn enabled(app: &tauri::AppHandle, addon: &Addon) -> Result<bool, String> {
    Ok(policy(app)
        .unwrap_or_default()
        .addons
        .get(addon.id)
        .is_some_and(|entry| entry.enabled))
}

pub(super) fn require_permissions(
    app: &tauri::AppHandle,
    addon: &Addon,
    path: &Path,
    version: &str,
    permissions: &[Permission],
) -> Result<(), String> {
    if !platform_supported() || !enabled(app, addon)? {
        return Err("addon is disabled or unavailable".into());
    }
    let declaration = installed(addon, path, version, true).map_err(manifest_problem)?;
    if !permissions
        .iter()
        .all(|permission| declaration.permissions.contains(permission))
    {
        return Err("the installed addon does not declare the required permissions".into());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    UnsupportedOs,
    Unpublished,
    Available,
    Enabled,
    Disabled,
    IncompatibleApi,
    VerificationFailed,
    NeedsReinstall,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerView {
    allow_direct_apply: bool,
    recovery: bool,
    addons: Vec<ManagedAddon>,
    auto_check: bool,
    registry: registry::CheckView,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedAddon {
    id: &'static str,
    name: LocalizedName,
    installed: Option<String>,
    previous: Option<String>,
    status: Status,
    expected_api: &'static str,
    found_api: Option<String>,
    permissions: Vec<Permission>,
    catalog_permissions: &'static [Permission],
    formats: &'static [&'static str],
    memory_bytes: usize,
    timeout_seconds: u64,
    output_bytes: usize,
    /// What actually confines the process: the AppContainer sandbox or Job Object limits only.
    isolation: &'static str,
    release: Option<ReleaseView>,
    update: Option<ReleaseView>,
    direct_apply: bool,
    activity: &'static str,
    progress: Option<(u64, u64)>,
    approval: Option<Approval>,
    problem: Option<InstallProblem>,
    summary: Option<registry::Summary>,
    repository: &'static str,
}

#[derive(Serialize)]
struct LocalizedName {
    en: String,
    de: String,
}

#[tauri::command]
pub async fn addon_list(
    app: tauri::AppHandle,
    importers: tauri::State<'_, Importers>,
) -> Result<ManagerView, String> {
    let policy = policy(&app);
    let recovery = policy.is_err();
    let policy = policy.unwrap_or_default();
    let registry = registry::current(&app).ok();
    let checks = app.state::<registry::Checks>().view(registry.as_ref());
    let mut addons = Vec::with_capacity(CATALOG.len());
    for &addon in CATALOG {
        let (job, _) = importers.get(addon.id)?;
        let dir = install_dir(&app, addon).map_err(|_| "installation-read-failed")?;
        let active = dir.active();
        let preference = policy.addons.get(addon.id).copied().unwrap_or_default();
        let active_version = active.as_ref().map(|(version, _)| version.as_str());
        let update = active_version
            .and_then(|_| select_release(registry.as_ref(), addon, active_version, true))
            .map(SelectedRelease::view);
        let release = select_release(registry.as_ref(), addon, active_version, false)
            .map(SelectedRelease::view);
        let mut status = if !platform_supported() {
            Status::UnsupportedOs
        } else if release.is_some() {
            Status::Available
        } else {
            Status::Unpublished
        };
        let mut found_api = None;
        let mut declaration = addon.declaration("");
        if platform_supported()
            && let Some((version, path)) = &active
        {
            status = match installed(addon, path, version, false) {
                Ok(found) => {
                    declaration = found;
                    if preference.enabled {
                        Status::Enabled
                    } else {
                        Status::Disabled
                    }
                }
                Err(Error::Incompatible(found)) => {
                    found_api = Some(found);
                    Status::IncompatibleApi
                }
                Err(Error::Invalid) => Status::VerificationFailed,
                Err(Error::NeedsReinstall) => Status::NeedsReinstall,
            };
        }
        let state = job.state.lock().unwrap();
        if active
            .as_ref()
            .is_some_and(|(version, _)| state.incompatible_session.as_ref() == Some(version))
        {
            status = Status::IncompatibleApi;
        }
        if state.startup_invalid {
            status = Status::VerificationFailed;
        }
        let (activity, progress) = match state.activity.as_ref().unwrap_or(&Activity::Idle) {
            Activity::Idle => ("idle", None),
            Activity::Checking => ("checking", None),
            Activity::Downloading { received, total } => ("downloading", Some((*received, *total))),
            Activity::Installing => ("installing", None),
            Activity::AwaitingApproval { .. } => ("awaitingApproval", None),
            Activity::InstallFailed { .. } => ("installFailed", None),
            Activity::Importing { .. } => ("importing", None),
            Activity::Publishing => ("publishing", None),
            Activity::Finished { .. } => ("finished", None),
            Activity::ImportFailed { .. } => ("importFailed", None),
        };
        let (formats, memory_bytes, timeout_seconds, output_bytes) = match addon.kind {
            Kind::Import { extensions, .. } => (
                extensions,
                JOB_MEMORY_BYTES,
                import::IMPORT_TIMEOUT.as_secs(),
                MAX_JSON_BYTES,
            ),
            Kind::Recognize { .. } => (
                &[][..],
                crate::recognition::math_process::MEMORY_BYTES,
                30,
                crate::recognition::math_process::LINE_LIMIT,
            ),
        };
        // The installed manifest's transport decides: legacy CLIs stay on the Job Object path
        // this beta, sessions run in the AppContainer.
        let isolation = if declaration.uses_session() {
            "sandbox"
        } else {
            "job"
        };
        addons.push(ManagedAddon {
            id: addon.id,
            name: LocalizedName {
                en: declaration.strings.en["name"].clone(),
                de: declaration.strings.de["name"].clone(),
            },
            previous: dir.previous().map(|(version, _)| version),
            installed: active.map(|(version, _)| version),
            status,
            expected_api: API,
            found_api,
            permissions: declaration.permissions,
            catalog_permissions: addon.permissions(),
            formats,
            memory_bytes,
            timeout_seconds,
            output_bytes,
            isolation,
            release,
            update,
            direct_apply: preference.direct_apply,
            activity,
            progress,
            problem: match &state.activity {
                Some(Activity::InstallFailed { problem, .. }) => Some(*problem),
                _ => None,
            },
            summary: registry
                .as_ref()
                .and_then(|registry| registry.docs(addon))
                .map(|docs| docs.summary.clone()),
            repository: addon.repository.trim_start_matches('/'),
            approval: match &state.activity {
                Some(Activity::AwaitingApproval { approval }) => Some(approval.clone()),
                _ => None,
            },
        });
    }
    Ok(ManagerView {
        allow_direct_apply: policy.allow_direct_apply,
        recovery,
        auto_check: !recovery && policy.auto_check,
        registry: checks,
        addons,
    })
}

#[tauri::command]
pub fn addon_reset_policy(app: tauri::AppHandle) -> Result<(), String> {
    let _lock = POLICY.lock().unwrap();
    let path = crate::settings::config_file(&app, CONFIG)?;
    if read_policy(&path).is_err() {
        write_policy(&path, &Policy::default()).map_err(|_| "policy-write-failed")?;
    }
    Ok(())
}

fn shutdown(app: &tauri::AppHandle, addon: &Addon) {
    if let Ok((job, _)) = app.state::<Importers>().get(addon.id) {
        let mut state = job.state.lock().unwrap();
        state.authorization_epoch += 1;
        state.incompatible_session = None;
    }
    app.state::<super::selection::Proposals>()
        .forget_addon(addon.id);
    super::session::revoke_grants(app, addon.id);
    if matches!(addon.kind, Kind::Recognize { .. }) {
        app.state::<super::session::Runtime>().shutdown();
        app.state::<crate::recognition::math_process::MathProcess>()
            .shutdown();
    }
}

#[tauri::command]
pub async fn addon_set_enabled(
    app: tauri::AppHandle,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_enabled(&app, id, enabled))
        .await
        .map_err(|_| "enable-write-failed")?
}

fn set_enabled(app: &tauri::AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let importers = app.state::<Importers>();
    let (job, addon) = importers.get(&id)?;
    if enabled {
        job.begin(Activity::Installing)
    } else {
        job.stop_import_and_reserve()
    }
    .map_err(|_| "busy")?;
    let result = (|| {
        if enabled {
            let (version, path) = install_dir(app, addon)?.active().ok_or("unavailable")?;
            if !platform_supported() {
                return Err("unsupported-os".into());
            }
            installed(addon, &path, &version, true).map_err(manifest_problem)?;
            job.state.lock().unwrap().startup_invalid = false;
        }
        patch(app, |policy| {
            policy.addons.entry(id).or_default().enabled = enabled;
        })?;
        if !enabled {
            shutdown(app, addon);
            job.state.lock().unwrap().source = None;
        }
        Ok(())
    })();
    job.set(Activity::Idle);
    result.map_err(|_: String| "enable-write-failed".into())
}

#[tauri::command]
pub async fn addon_remove(app: tauri::AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || remove(&app, &id))
        .await
        .map_err(|_| "remove-failed")?
}

fn remove(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let importers = app.state::<Importers>();
    let (job, addon) = importers.get(id)?;
    job.stop_import_and_reserve().map_err(|_| "busy")?;
    let result = (|| {
        shutdown(app, addon);
        let dir = install_dir(app, addon)?;
        let root = fs::canonicalize(app.path().app_local_data_dir().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let target = fs::canonicalize(&dir.0).map_err(|e| e.to_string())?;
        if target.parent() != Some(root.as_path()) || target.file_name() != dir.0.file_name() {
            return Err("invalid addon directory".into());
        }
        let removed = target.with_extension(format!("removed-{}", unique_suffix()));
        fs::rename(&target, &removed).map_err(|e| e.to_string())?;
        if let Err(error) = patch(app, |policy| {
            policy.addons.remove(addon.id);
        }) {
            fs::rename(&removed, &target).map_err(|e| e.to_string())?;
            return Err(error);
        }
        // Keep the job reserved until the retired directory has been pruned.
        let cleanup = fs::remove_dir_all(removed).map_err(|e| e.to_string());
        let mut state = job.state.lock().unwrap();
        // The epoch survives so a reinstall of the same version cannot revive old proposals.
        *state = State {
            activity: Some(Activity::Installing),
            authorization_epoch: state.authorization_epoch,
            ..State::default()
        };
        drop(state);
        cleanup?;
        Ok(())
    })();
    job.set(Activity::Idle);
    result.map_err(|_: String| "remove-failed".into())
}

#[tauri::command]
pub async fn addon_set_direct_apply(
    app: tauri::AppHandle,
    id: Option<String>,
    allowed: bool,
) -> Result<(), String> {
    let importers = app.state::<Importers>();
    if let Some(id) = &id {
        let (job, addon) = importers.get(id)?;
        let state = job.state.lock().unwrap();
        if state.busy() || install_dir(&app, addon)?.active().is_none() {
            return Err("unavailable".into());
        }
        return patch(&app, |policy| {
            policy.addons.entry(id.clone()).or_default().direct_apply = allowed;
        })
        .map_err(|_| "policy-write-failed".into());
    }
    patch(&app, |policy| {
        policy.allow_direct_apply = allowed;
    })
    .map_err(|_| "policy-write-failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_disabled_policy_from_backup_if_primary_is_deleted_or_corrupt() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(CONFIG);
        assert!(first_run(&path));
        patch_file(&path, |policy| {
            policy
                .addons
                .insert(catalog::GOODNOTES.id.into(), Preference::default());
        })
        .unwrap();
        fs::remove_file(&path).unwrap();
        assert!(!first_run(&path));
        assert!(!read_policy(&path).unwrap().addons[catalog::GOODNOTES.id].enabled);
        assert!(path.is_file());
        fs::write(&path, b"{").unwrap();
        assert!(!read_policy(&path).unwrap().addons[catalog::GOODNOTES.id].enabled);
        assert!(read_policy_file(&path).is_ok());
    }

    #[test]
    fn unrecoverable_policy_refuses_patches_until_explicit_reset() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(CONFIG);
        fs::write(&path, b"{").unwrap();
        assert!(!first_run(&path));
        assert!(read_policy(&path).is_err());
        fs::write(path.with_extension("json.bak"), b"{}").unwrap();
        assert!(patch_file(&path, |_| panic!("a corrupt policy must not be patched")).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{");
        write_policy(&path, &Policy::default()).unwrap();
        let policy = read_policy(&path).unwrap();
        assert!(!policy.allow_direct_apply && policy.addons.is_empty());
        assert!(policy.auto_check);
        assert!(!first_run(&path));
    }

    #[test]
    fn policy_defaults_and_independent_patches_survive_restart() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(CONFIG);
        assert!(!read_policy(&path).unwrap().allow_direct_apply);
        assert!(read_policy(&path).unwrap().auto_check);
        fs::write(&path, r#"{"allowDirectApply":false,"addons":{}}"#).unwrap();
        assert!(read_policy(&path).unwrap().auto_check);
        assert_eq!(
            Preference::default(),
            Preference {
                enabled: false,
                direct_apply: false
            }
        );

        std::thread::scope(|scope| {
            scope.spawn(|| patch_file(&path, |policy| policy.allow_direct_apply = true).unwrap());
            scope.spawn(|| patch_file(&path, |policy| policy.auto_check = false).unwrap());
            scope.spawn(|| {
                patch_file(&path, |policy| {
                    policy
                        .addons
                        .entry(catalog::GOODNOTES.id.into())
                        .or_default()
                        .enabled = true;
                })
                .unwrap()
            });
            scope.spawn(|| {
                patch_file(&path, |policy| {
                    policy
                        .addons
                        .entry(catalog::GOODNOTES.id.into())
                        .or_default()
                        .direct_apply = true;
                })
                .unwrap()
            });
        });
        let persisted = read_policy(&path).unwrap();
        assert!(persisted.allow_direct_apply);
        assert!(!persisted.auto_check);
        assert_eq!(
            persisted.addons[catalog::GOODNOTES.id],
            Preference {
                enabled: true,
                direct_apply: true
            }
        );

        patch_file(&path, |policy| {
            policy.addons.remove(catalog::GOODNOTES.id);
        })
        .unwrap();
        let persisted = read_policy(&path).unwrap();
        assert!(persisted.allow_direct_apply);
        assert!(!persisted.auto_check);
        assert!(persisted.addons.is_empty());

        fs::write(&path, b"{").unwrap();
        assert!(!read_policy(&path).unwrap().auto_check);
        assert!(!read_policy_file(&path).unwrap().auto_check);
    }

    #[test]
    fn preapproval_enables_repaired_addon_without_preserving_unverified_direct_apply() {
        let math = &catalog::MATH;
        let candidate = math.declaration("0.1.0");
        let prior = Preference {
            enabled: true,
            direct_apply: true,
        };

        assert!(requires_approval(None, &candidate, None, true));
        assert_eq!(
            install_preference(Some(prior), true, None, &candidate, false),
            Preference::default()
        );

        assert!(!requires_approval(
            Some(math.permissions()),
            &candidate,
            None,
            true
        ));
        assert_eq!(
            install_preference(Some(prior), true, None, &candidate, true),
            Preference {
                enabled: true,
                direct_apply: false
            }
        );

        // Previously installed scope never widens the user's approval for this attempt.
        assert!(requires_approval(
            Some(&[]),
            &candidate,
            Some(&candidate),
            true
        ));
    }

    #[test]
    fn preapproval_is_persisted_for_only_the_named_addon() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(CONFIG);
        let permissions = catalog::MATH.permissions().to_vec();
        patch_file(&path, |policy| {
            policy
                .pending_install_approvals
                .insert("math".into(), permissions.clone());
        })
        .unwrap();
        let saved = read_policy(&path).unwrap();
        assert_eq!(
            saved.pending_install_approvals,
            BTreeMap::from([("math".into(), permissions)])
        );
        assert!(saved.addons.is_empty());
    }

    #[test]
    fn a_scope_change_revokes_enablement_and_direct_apply_consent() {
        for addon in [&catalog::GOODNOTES, &catalog::ONENOTE] {
            let old = addon.declaration(addon.legacy.unwrap());
            let mut next = addon.declaration("9.9.9");
            let prior = Preference {
                enabled: true,
                direct_apply: true,
            };
            assert_eq!(
                install_preference(None, false, None, &next, false),
                Preference {
                    enabled: true,
                    direct_apply: false
                }
            );
            assert_eq!(
                install_preference(Some(prior), true, Some(&old), &next, false),
                prior
            );
            next.permissions.pop();
            assert_eq!(
                install_preference(Some(prior), true, Some(&old), &next, false),
                Preference::default()
            );
        }
    }
}
