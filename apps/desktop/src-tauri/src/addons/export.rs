use super::{contributions::Invocation, manifest::Surface, selection, session, *};
use base64::{Engine, engine::general_purpose::STANDARD};
use goodtype_core::{PageBackground, PageObject, storage};
use serde_json::{Value, json};
use tauri_plugin_dialog::DialogExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExportRequest {
    root: String,
    page_id: Option<String>,
    addon_id: String,
    command: String,
    language: String,
}

#[tauri::command]
pub(crate) async fn addon_export(
    app: tauri::AppHandle,
    request: ExportRequest,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || export(&app, request))
        .await
        .map_err(|_| "addon-unavailable")?
}

fn export(app: &tauri::AppHandle, request: ExportRequest) -> Result<bool, String> {
    let roots = app.state::<crate::workspace::AllowedRoots>();
    let root = crate::workspace::ensure_allowed(&roots, &request.root)?;
    let importers = app.state::<Importers>();
    let (job, addon) = importers.get(&request.addon_id)?;
    let authorization = selection::authorization(app, job, addon)?;
    let manifest = manager::installed(addon, &authorization.path, &authorization.version, true)
        .map_err(manager::manifest_problem)?;
    let format = manifest
        .commands
        .iter()
        .find(|command| command.id == request.command && command.surface == Surface::Export)
        .and_then(|command| command.format.as_ref())
        .ok_or("addon-unavailable")?;
    let strings = if request.language == "de" {
        &manifest.strings.de
    } else {
        &manifest.strings.en
    };
    let destination = app
        .dialog()
        .file()
        .add_filter(&strings[&format.label], &[format.extension.as_str()])
        .set_file_name(format!("notebook.{}", format.extension))
        .blocking_save_file()
        .map(|picked| picked.into_path().map_err(|_| "addon-export-failed"))
        .transpose()?;
    after_destination(destination, |destination| {
        let parent = destination.parent().ok_or("addon-export-failed")?;
        let parent = fs::canonicalize(parent).map_err(|_| "addon-export-failed")?;
        if parent.starts_with(&root) {
            return Err("addon-export-failed".into());
        }
        let histories = app.state::<crate::notebook::NotebookHistories>();
        let pages = crate::notebook::with_notebook(&histories, || {
            if crate::workspace::ensure_allowed(&roots, &request.root)? != root {
                return Err("addon-unavailable".into());
            }
            export_pages(&root, request.page_id.as_deref(), format.assets)
        })?;
        let reply = app
            .state::<session::Runtime>()
            .run(
                app,
                addon,
                &request.command,
                Invocation::Export {
                    assets: format.assets,
                    pages,
                },
                None,
                || selection::revoked(job, &authorization),
            )
            .map_err(|error| error.code())?
            .map_err(|error| selection::command_problem(error.code))?;
        let _idle = job.lock_idle().ok_or("addon-unavailable")?;
        if !selection::authorized(app, addon, &_idle).is_ok_and(|active| active == authorization) {
            return Err("addon-unavailable".into());
        }
        write_reply(
            &parent.join(destination.file_name().ok_or("addon-export-failed")?),
            reply,
        )?;
        Ok(())
    })
}

pub(super) fn after_destination(
    destination: Option<PathBuf>,
    publish: impl FnOnce(PathBuf) -> Result<(), String>,
) -> Result<bool, String> {
    let Some(destination) = destination else {
        return Ok(false);
    };
    publish(destination)?;
    Ok(true)
}

pub(super) fn export_pages(
    root: &Path,
    page_id: Option<&str>,
    assets: bool,
) -> Result<Vec<Value>, String> {
    let notebook = storage::open_notebook(root).map_err(|_| "addon-export-failed")?;
    let mut pages = Vec::new();
    let mut remaining = session::MAX_SCOPE;
    for page in notebook.manifest.pages {
        if page_id.is_some_and(|id| id != page.id) {
            continue;
        }
        let mut snapshot = storage::open_page(root, &page.id).map_err(|_| "addon-export-failed")?;
        if !assets {
            snapshot.page.objects.retain(|object| {
                !matches!(
                    object,
                    PageObject::Image { .. } | PageObject::PdfMaterial { .. }
                )
            });
            let ids: HashSet<_> = snapshot
                .page
                .objects
                .iter()
                .map(|object| object.fields().id.clone())
                .collect();
            snapshot.page.reading_order.retain(|id| ids.contains(id));
            for object in &mut snapshot.page.objects {
                if let PageObject::Group { child_ids, .. } = object {
                    child_ids.retain(|id| ids.contains(id));
                }
            }
            if matches!(snapshot.page.background, PageBackground::Pdf { .. }) {
                snapshot.page.background = PageBackground::Plain {
                    color: "#ffffff".into(),
                };
            }
            snapshot.assets.clear();
        }
        let references: HashSet<_> = snapshot
            .page
            .background
            .source()
            .into_iter()
            .chain(snapshot.page.objects.iter().filter_map(PageObject::source))
            .filter(|source| source.role == goodtype_core::SourceRole::Reference)
            .map(|source| source.path.to_owned())
            .collect();
        let mut page = serde_json::to_value(snapshot.page).map_err(|_| "addon-export-failed")?;
        for object in page["objects"]
            .as_array_mut()
            .ok_or("addon-export-failed")?
        {
            if let Some(source_file) = object["sourcePath"]
                .as_str()
                .and_then(|path| snapshot.blocks.iter().find(|block| block.path == path))
            {
                object
                    .as_object_mut()
                    .ok_or("addon-export-failed")?
                    .remove("sourcePath");
                object["source"] = Value::String(
                    String::from_utf8(source_file.bytes.clone())
                        .map_err(|_| "addon-export-failed")?,
                );
            }
        }
        page["inkLayers"] = json!(snapshot.ink_layers);
        if assets {
            let mut inline = serde_json::Map::new();
            let mut assets_budget = remaining;
            for asset in snapshot.assets {
                if asset.bytes.len() > assets_budget / 4 * 3 {
                    return Err("addon-export-too-large".into());
                }
                let encoded = STANDARD.encode(asset.bytes);
                assets_budget -= encoded.len();
                inline.insert(asset.path, Value::String(encoded));
            }
            for path in references {
                let bytes = storage::read_pdf_reference_bounded(root, &path, assets_budget / 4 * 3)
                    .map_err(|_| "addon-export-too-large")?;
                let encoded = STANDARD.encode(bytes);
                assets_budget -= encoded.len();
                inline.insert(path, Value::String(encoded));
            }
            page["assets"] = Value::Object(inline);
        }
        remaining = remaining
            .checked_sub(
                serde_json::to_vec(&page)
                    .map_err(|_| "addon-export-failed")?
                    .len(),
            )
            .ok_or("addon-export-too-large")?;
        pages.push(page);
    }
    if pages.is_empty() {
        return Err("addon-export-failed".into());
    }
    Ok(pages)
}

pub(super) fn write_reply(destination: &Path, reply: Value) -> Result<(), String> {
    let fields = reply
        .as_object()
        .filter(|fields| fields.len() == 1)
        .ok_or("addon-invalid-response")?;
    let encoded = fields
        .get("bytes")
        .and_then(Value::as_str)
        .filter(|bytes| bytes.len() <= session::MAX_SCOPE)
        .ok_or("addon-invalid-response")?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "addon-invalid-response")?;
    storage::files::persist_atomic(destination, &bytes, true)
        .map_err(|_| "addon-export-failed".into())
}
