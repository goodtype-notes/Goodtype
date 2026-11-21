//! Installation integration for the persistent recognition process.

use super::*;

pub(crate) use super::catalog::MATH;

#[tauri::command]
pub async fn remove_math_recognizer(app: tauri::AppHandle) -> Result<(), String> {
    manager::addon_remove(app, MATH.id.into()).await
}

pub(crate) fn with_math_installation<T>(
    app: &tauri::AppHandle,
    use_installation: impl FnOnce(Option<(PathBuf, String)>) -> Result<T, String>,
) -> Result<T, String> {
    let importers = app.state::<Importers>();
    let state = importers.get(MATH.id)?.0.lock_idle();
    // Lifecycle mutations reserve the same job before retiring a worker or changing activation.
    let installation = if platform_supported() && state.is_some() && manager::enabled(app, &MATH)? {
        if let Some((version, path)) = install_dir(app, &MATH)?.active() {
            manager::require_permissions(app, &MATH, &path, &version, MATH.permissions())?;
            // A session install runs only in its AppContainer; it never gets `--serve` here.
            if manager::installed(&MATH, &path, &version, false)
                .map_err(manager::manifest_problem)?
                .uses_session()
            {
                return Err("addon-unavailable".into());
            }
            let digest = installation_digest(&path)?;
            Some((path.join(MATH.executable()), digest))
        } else {
            None
        }
    } else {
        None
    };
    let result = use_installation(installation);
    drop(state);
    result
}
