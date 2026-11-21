use super::*;
use manifest::{API, Error, Manifest, Permission};
use std::collections::BTreeMap;

const CONFIG: &str = "addons.json";
const MAX_POLICY_BYTES: usize = 64 * 1024;
static POLICY: Mutex<()> = Mutex::new(());

#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct Policy {
    allow_direct_apply: bool,
    addons: BTreeMap<String, Preference>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct Preference {
    enabled: bool,
    direct_apply: bool,
}

fn read_policy(path: &Path) -> Result<Policy, String> {
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(Policy::default());
    }
    serde_json::from_slice(&read_bounded(path, MAX_POLICY_BYTES)?)
        .map_err(|error| error.to_string())
}

fn write_policy(path: &Path, policy: &Policy) -> Result<(), String> {
    crate::store::write_json(path, policy, MAX_POLICY_BYTES)
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
    check_contract(addon, &inventory, None, None).map_err(|_| Error::Invalid)?;
    manifest::load(addon, path, version, &inventory, None)
}

pub(crate) fn startup(app: &tauri::AppHandle) -> Result<(), String> {
    if !platform_supported() {
        return Ok(());
    }
    for addon in CATALOG {
        let dir = install_dir(app, addon)?;
        let Some((version, path)) = dir.active() else {
            continue;
        };
        match installed(addon, &path, &version, true) {
            Ok(_)
                if addon.legacy == Some(&version)
                    && !path.join(manifest::FILE).exists()
                    && !policy(app)?.addons.contains_key(addon.id) =>
            {
                patch(app, |policy| {
                    policy.addons.entry(addon.id.into()).or_insert(Preference {
                        enabled: true,
                        direct_apply: false,
                    });
                })?;
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
    Ok(())
}

pub(super) fn install(
    app: &tauri::AppHandle,
    dir: &InstallDir,
    archive: &Path,
    release: &PinnedRelease,
) -> Result<String, String> {
    let previous = dir.active();
    let previous_declaration = previous
        .as_ref()
        .and_then(|(version, path)| installed(dir.1, path, version, true).ok());
    install_with_policy(
        dir,
        archive,
        release,
        &|path| probe_tool(dir.1, path),
        |candidate| {
            let _lock = POLICY.lock().unwrap();
            let path = crate::settings::config_file(app, CONFIG)?;
            let mut policy = read_policy(&path)?;
            let prior = policy.addons.get(dir.1.id).copied();
            let next = install_preference(
                prior,
                previous.is_some(),
                previous_declaration.as_ref(),
                candidate,
            );
            if Some(next) == prior {
                return dir.activate(release.version);
            }
            policy.addons.insert(dir.1.id.into(), next);
            write_policy(&path, &policy)?;
            if let Err(error) = dir.activate(release.version) {
                if let Some(prior) = prior {
                    policy.addons.insert(dir.1.id.into(), prior);
                } else {
                    policy.addons.remove(dir.1.id);
                }
                write_policy(&path, &policy)?;
                return Err(error);
            }
            Ok(())
        },
    )
}

fn install_preference(
    prior: Option<Preference>,
    had_install: bool,
    previous: Option<&Manifest>,
    candidate: &Manifest,
) -> Preference {
    if !had_install {
        Preference {
            enabled: true,
            direct_apply: false,
        }
    } else if previous.is_some_and(|old| old.same_scope(candidate)) {
        prior.unwrap_or_default()
    } else {
        Preference::default()
    }
}

pub(super) fn enabled(app: &tauri::AppHandle, addon: &Addon) -> Result<bool, String> {
    Ok(policy(app)?
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerView {
    allow_direct_apply: bool,
    addons: Vec<ManagedAddon>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedAddon {
    id: &'static str,
    name: LocalizedName,
    installed: Option<String>,
    status: Status,
    expected_api: &'static str,
    found_api: Option<String>,
    permissions: Vec<Permission>,
    catalog_permissions: &'static [Permission],
    formats: &'static [&'static str],
    memory_bytes: usize,
    timeout_seconds: u64,
    output_bytes: usize,
    release: Option<ReleaseView>,
    update: Option<ReleaseView>,
    update_checked: bool,
    update_failed: bool,
    can_check_update: bool,
    direct_apply: bool,
    activity: &'static str,
    progress: Option<(u64, u64)>,
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
    let policy = policy(&app).map_err(|_| "policy-read-failed")?;
    let mut addons = Vec::with_capacity(CATALOG.len());
    for &addon in CATALOG {
        let (job, _) = importers.get(addon.id)?;
        let dir = install_dir(&app, addon).map_err(|_| "installation-read-failed")?;
        let active = dir.active();
        let preference = policy.addons.get(addon.id).copied().unwrap_or_default();
        let mut status = if !platform_supported() {
            Status::UnsupportedOs
        } else if addon.pinned.is_some() {
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
            };
        }
        let state = job.state.lock().unwrap();
        if state.startup_invalid {
            status = Status::VerificationFailed;
        }
        let (activity, progress) = match state.activity.as_ref().unwrap_or(&Activity::Idle) {
            Activity::Idle => ("idle", None),
            Activity::Checking => ("checking", None),
            Activity::Downloading { received, total } => ("downloading", Some((*received, *total))),
            Activity::Installing => ("installing", None),
            Activity::InstallFailed { message } if message == "incompatible addon API" => {
                ("updateIncompatible", None)
            }
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
        addons.push(ManagedAddon {
            id: addon.id,
            name: LocalizedName {
                en: declaration.strings.en["name"].clone(),
                de: declaration.strings.de["name"].clone(),
            },
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
            release: addon.pinned.map(|pin| ReleaseView {
                version: pin.version.into(),
                size_bytes: pin.size,
            }),
            update: state.update.as_ref().map(|release| release.view()),
            update_checked: state.update_checked,
            update_failed: state.update_problem.is_some(),
            can_check_update: addon.public_key.is_some(),
            direct_apply: preference.direct_apply,
            activity,
            progress,
        });
    }
    Ok(ManagerView {
        allow_direct_apply: policy.allow_direct_apply,
        addons,
    })
}

fn shutdown(app: &tauri::AppHandle, addon: &Addon) {
    if matches!(addon.kind, Kind::Recognize { .. }) {
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
        *job.state.lock().unwrap() = State {
            activity: Some(Activity::Installing),
            ..State::default()
        };
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
    fn policy_defaults_and_independent_patches_survive_restart() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(CONFIG);
        assert!(!read_policy(&path).unwrap().allow_direct_apply);
        assert_eq!(
            Preference::default(),
            Preference {
                enabled: false,
                direct_apply: false
            }
        );

        std::thread::scope(|scope| {
            scope.spawn(|| patch_file(&path, |policy| policy.allow_direct_apply = true).unwrap());
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
        assert!(persisted.addons.is_empty());
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
                install_preference(None, false, None, &next),
                Preference {
                    enabled: true,
                    direct_apply: false
                }
            );
            assert_eq!(
                install_preference(Some(prior), true, Some(&old), &next),
                prior
            );
            next.permissions.pop();
            assert_eq!(
                install_preference(Some(prior), true, Some(&old), &next),
                Preference::default()
            );
        }
    }
}
