//! Installation integration for the persistent recognition process.

use super::*;

pub(crate) use super::catalog::MATH;

/// Remove only this fixed app-owned add-on directory; never a frontend path.
#[tauri::command]
pub async fn remove_math_recognizer(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let importers = app.state::<Importers>();
        let math = app.state::<crate::recognition::math_process::MathProcess>();
        let (importer, tool) = importers.get(MATH.id)?;
        importer.begin(Activity::Installing)?;
        let result = (|| {
            math.shutdown();
            let dir = install_dir(&app, tool)?;
            let root =
                fs::canonicalize(app.path().app_local_data_dir().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let target = fs::canonicalize(&dir.0).map_err(|e| e.to_string())?;
            if target.parent() != Some(root.as_path()) {
                return Err("invalid add-on directory".into());
            }
            fs::remove_dir_all(target).map_err(|e| e.to_string())
        })();
        importer.set(Activity::Idle);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn math_executable(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    if app.state::<Importers>().get(MATH.id)?.0.busy() {
        return Err("math recognizer installation is busy".into());
    }
    let dir = install_dir(app, &MATH)?;
    let Some((_, path)) = installed_contract(&dir)? else {
        return Ok(None);
    };
    let manifest = verify_package(&MATH, &path)?;
    check_contract(&MATH, &manifest, None, MATH.pinned.as_ref())?;
    Ok(Some(path.join(MATH.executable())))
}
