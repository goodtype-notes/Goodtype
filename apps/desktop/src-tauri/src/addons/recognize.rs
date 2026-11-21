//! Installation integration for the persistent recognition process.

use super::*;

pub(crate) use super::catalog::MATH;

#[tauri::command]
pub async fn remove_math_recognizer(app: tauri::AppHandle) -> Result<(), String> {
    manager::addon_remove(app, MATH.id.into()).await
}

pub(crate) fn math_available(app: &tauri::AppHandle) -> Result<bool, String> {
    if !platform_supported() || app.state::<Importers>().get(MATH.id)?.0.busy() {
        return Ok(false);
    }
    if !manager::enabled(app, &MATH)? {
        return Ok(false);
    }
    let Some((version, path)) = install_dir(app, &MATH)?.active() else {
        return Ok(false);
    };
    let declaration =
        manager::installed(&MATH, &path, &version, false).map_err(manager::manifest_problem)?;
    Ok(MATH
        .permissions()
        .iter()
        .all(|permission| declaration.permissions.contains(permission)))
}

pub(crate) fn math_executable(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    if !math_available(app)? {
        return Ok(None);
    }
    let dir = install_dir(app, &MATH)?;
    let Some((version, path)) = dir.active() else {
        return Ok(None);
    };
    manager::require_permissions(app, &MATH, &path, &version, MATH.permissions())?;
    Ok(Some(path.join(MATH.executable())))
}
