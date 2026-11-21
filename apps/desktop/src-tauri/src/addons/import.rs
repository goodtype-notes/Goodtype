//! Notebook import capability: native source selection, conversion and Library publication.

use goodtype_core::{layout, storage};
use tauri_plugin_dialog::DialogExt;

use crate::{library, workspace::AllowedRoots};

use super::*;

pub(super) const IMPORT_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);
const MAX_LISTED_WARNINGS: usize = 12;

/// What became of one notebook from the source.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookOutcome {
    /// Name or folder path inside the source export.
    pub source: String,
    /// `imported`, `failed`, `needsArtwork`, `cancelled` or `alreadyThere`.
    pub status: &'static str,
    /// Library-relative path of the imported notebook.
    pub path: Option<String>,
    pub pages: usize,
    pub error: Option<String>,
    /// Page-level notes from the importer, e.g. a feature it could not carry over.
    pub warnings: Vec<String>,
    /// Pages whose content became vector artwork with reduced editability.
    pub artwork_pages: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// Library-relative batch folder, if anything was imported into it.
    pub batch: Option<String>,
    pub notebooks: Vec<NotebookOutcome>,
    pub cancelled: bool,
    /// Process or report problem not tied to one notebook.
    pub problem: Option<String>,
}

/// A notebook folder the importer says it completed must still be an ordinary, valid notebook:
/// no interrupted transaction (opening would "recover" it, i.e. write), manifest and every page
/// readable through the same core the app uses.
pub fn validate_notebook(dir: &Path) -> Result<(), String> {
    if dir.join(layout::PENDING_TRANSACTION).exists()
        || dir.join(layout::LEGACY_PENDING_TRANSACTION).exists()
    {
        return Err("the notebook has an unfinished transaction".into());
    }
    let snapshot = storage::open_notebook(dir).map_err(|error| error.to_string())?;
    for page in &snapshot.manifest.pages {
        storage::open_page(dir, &page.id).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct NotebookReport {
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    pages: Vec<PageReport>,
}

#[derive(Deserialize)]
struct PageReport {
    page: usize,
    #[serde(default)]
    derived: Vec<serde_json::Value>,
    #[serde(default)]
    warnings: Vec<String>,
}

/// Bounded page notes from a notebook's own report, for the result list.
fn notebook_notes(dir: &Path) -> (Vec<String>, usize) {
    let Ok(bytes) = read_bounded(&dir.join("import-report.json"), MAX_JSON_BYTES) else {
        return (Vec::new(), 0);
    };
    let Ok(report) = serde_json::from_slice::<NotebookReport>(&bytes) else {
        return (Vec::new(), 0);
    };
    let mut notes: Vec<String> = report.warnings;
    for page in &report.pages {
        notes.extend(
            page.warnings
                .iter()
                .map(|warning| format!("Page {}: {warning}", page.page)),
        );
    }
    notes.truncate(MAX_LISTED_WARNINGS);
    let artwork = report
        .pages
        .iter()
        .filter(|page| !page.derived.is_empty())
        .count();
    (notes, artwork)
}

fn needs_artwork(error: &str) -> bool {
    error.contains("--allow-derived")
}

/// Moves validated, completed notebooks from the tool's batch into the Library batch folder.
/// `library_batch` is `(absolute folder, library-relative path)`; it is created on the first
/// published notebook. Never replaces anything already there.
pub fn publish(
    entries: &[ReportEntry],
    tool_batch: &Path,
    library_batch: (&Path, &str),
    cancelled: bool,
) -> Vec<NotebookOutcome> {
    let (folder, relative_folder) = library_batch;
    entries
        .iter()
        .map(|entry| {
            let outcome = |status, error: Option<String>| NotebookOutcome {
                source: entry.source.clone(),
                status,
                path: None,
                pages: entry.pages,
                error,
                warnings: Vec::new(),
                artwork_pages: 0,
            };
            match entry.status.as_str() {
                "completed" => {}
                "failed" => {
                    let error = entry.error.clone().unwrap_or_default();
                    let status = if needs_artwork(&error) {
                        "needsArtwork"
                    } else {
                        "failed"
                    };
                    return outcome(status, Some(error));
                }
                _ => {
                    return outcome(
                        if cancelled { "cancelled" } else { "failed" },
                        (!cancelled).then(|| "the importer stopped before this notebook".into()),
                    );
                }
            }
            let Some(target) = entry.target.as_deref() else {
                return outcome("failed", Some("the report names no folder".into()));
            };
            let components = match safe_components(target) {
                Ok(components) => components,
                Err(error) => return outcome("failed", Some(error)),
            };
            let produced = components
                .iter()
                .fold(tool_batch.to_path_buf(), |path, part| path.join(part));
            let destination = components
                .iter()
                .fold(folder.to_path_buf(), |path, part| path.join(part));
            if destination.exists() {
                let mut kept = outcome("alreadyThere", None);
                kept.path = Some(format!("{relative_folder}/{}", components.join("/")));
                return kept;
            }
            if let Err(error) = validate_notebook(&produced) {
                return outcome(
                    "failed",
                    Some(format!("Goodtype could not open the result: {error}")),
                );
            }
            let moved = destination
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::rename(&produced, &destination));
            if let Err(error) = moved {
                return outcome(
                    "failed",
                    Some(format!("could not place the notebook: {error}")),
                );
            }
            let (warnings, artwork_pages) = notebook_notes(&destination);
            NotebookOutcome {
                source: entry.source.clone(),
                status: "imported",
                path: Some(format!("{relative_folder}/{}", components.join("/"))),
                pages: entry.pages,
                error: None,
                warnings,
                artwork_pages,
            }
        })
        .collect()
}

/// `<batch> 2026-10-21`, or the first free ` (2)`, ` (3)`… beside it.
pub(super) fn batch_name(tool: &Addon, parent: &Path, today: &str) -> Result<String, String> {
    let Kind::Import { batch, .. } = tool.kind else {
        return Err("this tool does not import notebooks".into());
    };
    let base = format!("{batch} {today}");
    (1..1000)
        .map(|number| {
            if number == 1 {
                base.clone()
            } else {
                format!("{base} ({number})")
            }
        })
        .find(|name| !parent.join(name).exists())
        .ok_or_else(|| "no free folder name for the import".into())
}

/// Today's date (UTC) as `YYYY-MM-DD`, without a calendar dependency.
pub(super) fn today() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Native file picker for the importer's source files. The path stays here.
#[tauri::command]
pub async fn import_pick_source(
    app: tauri::AppHandle,
    importers: tauri::State<'_, Importers>,
    tool: String,
    ui_language: Option<String>,
) -> Result<Option<SourceView>, String> {
    let (importer, tool) = importers.get(&tool)?;
    if !platform_supported()
        || install_dir(&app, tool)?.active().is_some() && !manager::enabled(&app, tool)?
    {
        return Err("addon is disabled or unavailable".into());
    }
    let Kind::Import {
        filter, extensions, ..
    } = tool.kind
    else {
        return Err("this tool does not import notebooks".into());
    };
    let Some(picked) = app
        .dialog()
        .file()
        .set_title(crate::settings::dialog_label(
            ui_language.as_deref(),
            "Choose source file",
            "Quelldatei auswählen",
        ))
        .add_filter(filter, extensions)
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|error| error.to_string())?;
    let accepted = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(extension))
        });
    if !path.is_file() || !accepted {
        return Err(format!(
            "choose a {filter} file ({})",
            extensions
                .iter()
                .map(|extension| format!(".{extension}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let view = source_view(&path).ok_or("that file cannot be read")?;
    let mut state = importer.state.lock().unwrap();
    if state.activity.as_ref().is_some_and(Activity::is_busy)
        || install_dir(&app, tool)?.active().is_some() && !manager::enabled(&app, tool)?
    {
        return Err("addon is disabled or busy".into());
    }
    state.source = Some(path);
    Ok(Some(view))
}

/// Imports the picked source into a new batch folder inside the Library folder `parent`, or,
/// with `into`, adds what is missing to an earlier batch (a retry with vector artwork allowed).
#[tauri::command]
pub fn import_start(
    app: tauri::AppHandle,
    roots: tauri::State<'_, AllowedRoots>,
    importers: tauri::State<'_, Importers>,
    tool: String,
    parent: String,
    allow_derived: bool,
    into: Option<String>,
) -> Result<(), String> {
    let (importer, tool) = importers.get(&tool)?;
    if importer.busy() {
        return Err("the importer is busy".into());
    }
    let source = importer
        .state
        .lock()
        .unwrap()
        .source
        .clone()
        .ok_or("choose a file first")?;
    let root = library::current_root(&app, &roots)?;
    let parent_dir = library::resolve(&root, &parent)?;
    if library::is_notebook(&parent_dir) {
        return Err("choose a Library folder, not a notebook".into());
    }
    library::reject_notebook_internals(&root, &parent_dir)?;
    let (batch_dir, batch_relative) = match into {
        Some(existing) => {
            let folder = library::resolve(&root, &existing)?;
            if !folder.is_dir() || library::is_notebook(&folder) {
                return Err("the earlier import folder is gone".into());
            }
            library::reject_notebook_internals(&root, &folder)?;
            (folder, existing)
        }
        None => {
            let name = batch_name(tool, &parent_dir, &today())?;
            library::validate_name(&name)?;
            let relative = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            (parent_dir.join(&name), relative)
        }
    };
    // Hidden from the Library listing (dot name) and on the destination volume, so publication is
    // a rename. The tool creates its own scratch folder beside its batch, i.e. inside this one.
    importer.begin(Activity::Importing {
        done: 0,
        total: 0,
        cancelling: false,
    })?;
    let installation = (|| {
        let (version, path) = install_dir(&app, tool)?
            .active()
            .ok_or("the importer is not installed")?;
        let mut permissions = vec![
            manifest::Permission::PickedImportFile,
            manifest::Permission::PublishImportBatch,
        ];
        if allow_derived {
            permissions.push(manifest::Permission::DerivedImportArtwork);
        }
        manager::require_permissions(&app, tool, &path, &version, &permissions)?;
        if importer.state.lock().unwrap().source.as_ref() != Some(&source) {
            return Err("the chosen source changed".into());
        }
        Ok(path)
    })();
    let installed = match installation {
        Ok(path) => path,
        Err(error) => {
            importer.set(Activity::Idle);
            return Err(error);
        }
    };
    let staging = parent_dir.join(format!(".goodtype-import-{}", unique_suffix()));
    if let Err(error) = fs::create_dir(&staging) {
        importer.set(Activity::Idle);
        return Err(error.to_string());
    }
    let state = importer.state.clone();
    let cancel = importer.cancel.clone();
    std::thread::spawn(move || {
        let result = run_import(
            tool,
            &installed,
            &source,
            &staging,
            (&batch_dir, &batch_relative),
            allow_derived,
            &cancel,
            &state,
        );
        let _ = fs::remove_dir_all(&staging);
        state.lock().unwrap().activity = Some(match result {
            Ok(result) => Activity::Finished { result },
            Err(message) => Activity::ImportFailed { message },
        });
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_import(
    tool: &Addon,
    installed: &Path,
    source: &Path,
    staging: &Path,
    library_batch: (&Path, &str),
    allow_derived: bool,
    cancel: &AtomicBool,
    state: &Mutex<State>,
) -> Result<ImportResult, String> {
    // The files on disk are checked again right before they run.
    verify_package(tool, installed).map_err(|error| {
        format!("the installed importer is damaged ({error}); download it again")
    })?;
    let tool_batch = staging.join("batch");
    let report = tool_batch.join("import-report.json");
    let mut command = Command::new(installed.join(tool.executable()));
    command.arg(source).arg(&tool_batch).current_dir(staging);
    if allow_derived {
        command.arg("--allow-derived");
    }
    let finished = run_contained(&mut command, IMPORT_TIMEOUT, cancel, &|| {
        if let Ok(entries) = read_report(&report) {
            let done = entries
                .iter()
                .filter(|entry| entry.status != "pending")
                .count();
            if let Some(Activity::Importing {
                done: shown, total, ..
            }) = state.lock().unwrap().activity.as_mut()
            {
                *shown = done;
                *total = entries.len();
            }
        }
    })?;
    state.lock().unwrap().activity = Some(Activity::Publishing);
    let entries = match read_report(&report) {
        Ok(entries) => entries,
        Err(_) if finished.cancelled => Vec::new(),
        Err(error) => {
            let detail = finished.stderr.trim();
            return Err(if detail.is_empty() {
                format!("the importer produced no usable report ({error})")
            } else {
                detail.lines().last().unwrap_or(detail).to_owned()
            });
        }
    };
    let created = !library_batch.0.exists();
    let notebooks = publish(&entries, &tool_batch, library_batch, finished.cancelled);
    let published = notebooks
        .iter()
        .any(|notebook| notebook.status == "imported");
    if created && !published && library_batch.0.exists() {
        let _ = fs::remove_dir(library_batch.0);
    }
    let problem = (!finished.success
        && !finished.cancelled
        && entries.iter().all(|entry| entry.status != "failed"))
    .then(|| {
        finished
            .stderr
            .trim()
            .lines()
            .last()
            .unwrap_or("the importer stopped unexpectedly")
            .to_owned()
    });
    Ok(ImportResult {
        batch: library_batch.0.exists().then(|| library_batch.1.to_owned()),
        notebooks,
        cancelled: finished.cancelled,
        problem,
    })
}
