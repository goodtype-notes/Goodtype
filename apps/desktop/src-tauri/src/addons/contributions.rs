use super::{manifest::identifier, *};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) type Values = BTreeMap<String, Value>;

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Choice {
    value: String,
    label: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(super) enum Field {
    Toggle {
        id: String,
        label: String,
        hint: Option<String>,
        search: Option<String>,
        default: bool,
    },
    Choice {
        id: String,
        label: String,
        hint: Option<String>,
        search: Option<String>,
        default: String,
        choices: Vec<Choice>,
    },
    Text {
        id: String,
        label: String,
        hint: Option<String>,
        search: Option<String>,
        #[serde(rename = "maxLength")]
        max_length: usize,
        pattern: Option<Pattern>,
        #[serde(default)]
        default: String,
    },
    Number {
        id: String,
        label: String,
        hint: Option<String>,
        search: Option<String>,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
    },
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Pattern {
    Any,
    Number,
    Identifier,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Format {
    pub extension: String,
    pub label: String,
    #[serde(default)]
    pub assets: bool,
}

impl Format {
    pub fn valid(&self, strings: &BTreeMap<String, String>) -> bool {
        (1..=8).contains(&self.extension.len())
            && self.extension != "goodtype"
            && self
                .extension
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && strings.contains_key(&self.label)
    }
}

impl Field {
    fn metadata(&self) -> (&str, &str, Option<&str>, Option<&str>) {
        match self {
            Self::Toggle {
                id,
                label,
                hint,
                search,
                ..
            }
            | Self::Choice {
                id,
                label,
                hint,
                search,
                ..
            }
            | Self::Text {
                id,
                label,
                hint,
                search,
                ..
            }
            | Self::Number {
                id,
                label,
                hint,
                search,
                ..
            } => (id, label, hint.as_deref(), search.as_deref()),
        }
    }

    pub fn id(&self) -> &str {
        self.metadata().0
    }

    pub fn default_value(&self) -> Value {
        match self {
            Self::Toggle { default, .. } => Value::from(*default),
            Self::Choice { default, .. } | Self::Text { default, .. } => {
                Value::from(default.clone())
            }
            Self::Number { default, .. } => Value::from(*default),
        }
    }

    pub fn accepts(&self, value: &Value) -> bool {
        match self {
            Self::Toggle { .. } => value.is_boolean(),
            Self::Choice { choices, .. } => value
                .as_str()
                .is_some_and(|s| choices.iter().any(|c| c.value == s)),
            Self::Text {
                max_length,
                pattern,
                ..
            } => value.as_str().is_some_and(|s| {
                s.chars().count() <= *max_length
                    && !s.chars().any(char::is_control)
                    && match pattern.unwrap_or(Pattern::Any) {
                        Pattern::Any => true,
                        Pattern::Number => s.parse::<f64>().is_ok_and(f64::is_finite),
                        Pattern::Identifier => identifier(s),
                    }
            }),
            Self::Number { min, max, step, .. } => value.as_f64().is_some_and(|n| {
                let ticks = (n - min) / step;
                n.is_finite() && (*min..=*max).contains(&n) && (ticks - ticks.round()).abs() <= 1e-8
            }),
        }
    }
}

pub(super) fn valid_fields(
    fields: &[Field],
    strings: &BTreeMap<String, String>,
    settings: bool,
) -> bool {
    let mut ids = HashSet::new();
    fields.len() <= if settings { 16 } else { 12 }
        && fields.iter().all(|field| {
            let (id, label, hint, search) = field.metadata();
            identifier(id)
                && ids.insert(id)
                && [Some(label), hint, search]
                    .into_iter()
                    .flatten()
                    .all(|key| strings.contains_key(key))
                && match field {
                    Field::Toggle { .. } => true,
                    Field::Choice { choices, .. } => {
                        let mut values = HashSet::new();
                        (2..=8).contains(&choices.len())
                            && choices.iter().all(|choice| {
                                identifier(&choice.value)
                                    && values.insert(&choice.value)
                                    && strings.contains_key(&choice.label)
                            })
                    }
                    Field::Text {
                        max_length,
                        default,
                        ..
                    } => {
                        !settings
                            && (1..=512).contains(max_length)
                            && (default.is_empty() || field.accepts(&field.default_value()))
                    }
                    Field::Number { min, max, step, .. } => !settings && min < max && *step > 0.0,
                }
                && (matches!(field, Field::Text { .. }) || field.accepts(&field.default_value()))
        })
}

pub(super) fn validate_values(fields: &[Field], values: &Values) -> bool {
    fields.len() == values.len()
        && fields.iter().all(|field| {
            values
                .get(field.id())
                .is_some_and(|value| field.accepts(value))
        })
}

pub(super) fn defaults(fields: &[Field]) -> Values {
    fields
        .iter()
        .map(|field| (field.id().into(), field.default_value()))
        .collect()
}

use super::manifest::{Command, Strings, Surface};
use super::session::Failure;
use serde_json::json;

pub(super) enum Invocation {
    Selection(Value),
    OwnObjects(Vec<Value>),
    Reaction(Value),
    Tool {
        points: usize,
        snapped: bool,
        params: Value,
    },
    Form {
        surface: Surface,
        values: Values,
        geometry: goodtype_core::PageGeometry,
        anchor: [f64; 2],
    },
    Export {
        assets: bool,
        pages: Vec<Value>,
    },
}

impl Invocation {
    pub fn surface(&self) -> Surface {
        match self {
            Self::Selection(_) => Surface::InkSelection,
            Self::OwnObjects(_) | Self::Tool { .. } | Self::Reaction(_) => Surface::OwnObject,
            Self::Form { surface, .. } => *surface,
            Self::Export { .. } => Surface::Export,
        }
    }

    pub fn params(self, command: &Command, settings: Values) -> Result<Value, Failure> {
        let id = &command.id;
        match self {
            Self::OwnObjects(objects) => {
                Ok(json!({"command": id, "settings": settings, "objects": objects}))
            }
            Self::Tool { .. } | Self::Reaction(_) => Err(Failure::Unavailable),
            Self::Selection(scope) => {
                Ok(json!({"command": id, "settings": settings, "scope": scope}))
            }
            Self::Form {
                values,
                geometry,
                anchor: [x, y],
                ..
            } => {
                if !command
                    .form
                    .as_ref()
                    .is_some_and(|fields| validate_values(fields, &values))
                {
                    return Err(Failure::InvalidForm);
                }
                Ok(
                    json!({"command": id, "settings": settings, "form": values, "page": geometry, "anchor": {"x": x, "y": y}}),
                )
            }
            Self::Export { assets, pages } => {
                if !command
                    .format
                    .as_ref()
                    .is_some_and(|format| format.assets == assets)
                {
                    return Err(Failure::Unavailable);
                }
                Ok(json!({"command": id, "settings": settings, "pages": pages}))
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContributionView {
    addon_id: String,
    enabled: bool,
    direct_apply: bool,
    commands: Vec<Command>,
    tools: Vec<super::manifest::Tool>,
    strings: Strings,
    settings: Vec<Field>,
    values: Values,
}

#[tauri::command]
pub(crate) async fn addon_contributions(
    app: tauri::AppHandle,
) -> Result<Vec<ContributionView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let importers = app.state::<Importers>();
        let mut views = Vec::new();
        for &addon in CATALOG {
            let (job, _) = importers.get(addon.id)?;
            let Some(idle) = job.lock_idle() else {
                continue;
            };
            let Some((version, path)) = install_dir(&app, addon)?.active() else {
                continue;
            };
            if idle.incompatible_session.as_deref() == Some(&version) {
                continue;
            }
            let Ok(manifest) = manager::installed(addon, &path, &version, false) else {
                continue;
            };
            let commands: Vec<_> = manifest
                .commands
                .into_iter()
                .filter(|command| {
                    matches!(
                        command.surface,
                        Surface::InsertForm
                            | Surface::PageGenerator
                            | Surface::Export
                            | Surface::OwnObject
                    )
                })
                .collect();
            if commands.is_empty() && manifest.settings.is_empty() && manifest.tools.is_empty() {
                continue;
            }
            views.push(ContributionView {
                addon_id: addon.id.into(),
                enabled: manager::enabled(&app, addon)?,
                direct_apply: manager::direct_apply(&app, addon.id),
                values: manager::settings(&app, addon.id, &manifest.settings)?,
                settings: manifest.settings,
                strings: manifest.strings,
                commands,
                tools: manifest.tools,
            });
        }
        Ok(views)
    })
    .await
    .map_err(|_| "addon-unavailable")?
}

use super::{
    batch::{self, PreviewPage},
    selection, session,
};
use crate::{
    notebook::{self, NotebookError, NotebookHistories},
    workspace::{self, AllowedRoots},
};
use goodtype_core::{PageGeometry, storage};

struct PendingForm {
    root: PathBuf,
    page_id: String,
    base: storage::ProposalBase,
    addon_id: String,
    authorization: selection::Authorization,
    direct: bool,
    generator: bool,
    geometry: PageGeometry,
    pages: Vec<PreviewPage>,
}

#[derive(Default)]
pub(crate) struct PendingForms(Mutex<HashMap<String, PendingForm>>);

impl PendingForms {
    pub(crate) fn forget_addon(&self, id: &str) {
        self.0.lock().unwrap().retain(|_, form| form.addon_id != id);
    }
    pub(crate) fn forget_root(&self, root: &Path) {
        self.0.lock().unwrap().retain(|_, form| form.root != root);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct FormRequest {
    root: String,
    page_id: String,
    addon_id: String,
    command: String,
    request_id: String,
    review_first: bool,
    form: Values,
    geometry: Option<PageGeometry>,
    anchor: [f64; 2],
}

#[derive(Serialize)]
pub(crate) struct FormPreview {
    id: String,
    direct: bool,
    geometry: PageGeometry,
    pages: Vec<PreviewPage>,
}

#[tauri::command]
pub(crate) async fn addon_form_preview(
    app: tauri::AppHandle,
    request: FormRequest,
) -> Result<FormPreview, String> {
    let cancellation = app
        .state::<selection::Proposals>()
        .begin(&request.request_id)?;
    let id = request.request_id.clone();
    let handle = app.clone();
    let result =
        tauri::async_runtime::spawn_blocking(move || preview(&handle, request, &cancellation))
            .await;
    app.state::<selection::Proposals>().finish(&id);
    result.map_err(|_| "addon-unavailable")?
}

fn preview(
    app: &tauri::AppHandle,
    request: FormRequest,
    cancelled: &AtomicBool,
) -> Result<FormPreview, String> {
    let roots = app.state::<AllowedRoots>();
    let root = workspace::ensure_allowed(&roots, &request.root)?;
    let histories = app.state::<NotebookHistories>();
    let importers = app.state::<Importers>();
    let (job, addon) = importers.get(&request.addon_id)?;
    let authorization = selection::authorization(app, job, addon)?;
    let generator = request.geometry.is_some();
    let (base, geometry) = notebook::with_notebook(&histories, || {
        let snapshot =
            storage::open_page(&root, &request.page_id).map_err(|_| "addon-unavailable")?;
        if !generator && snapshot.page.calculation.is_some() {
            return Err("addon-unavailable".into());
        }
        let base =
            storage::proposal_base(&root, &snapshot, &[]).map_err(|_| "addon-unavailable")?;
        Ok((base, request.geometry.unwrap_or(snapshot.page.geometry)))
    })?;
    let [x, y] = request.anchor;
    if ![geometry.width_pt, geometry.height_pt]
        .into_iter()
        .all(|dim| dim > 0.0 && dim <= goodtype_core::MAX_PAGE_DIMENSION_PT)
        || !(0.0..=geometry.width_pt).contains(&x)
        || !(0.0..=geometry.height_pt).contains(&y)
    {
        return Err("addon-invalid-form".into());
    }
    let surface = if generator {
        Surface::PageGenerator
    } else {
        Surface::InsertForm
    };
    let direct = !generator && !request.review_first && manager::direct_apply(app, addon.id);
    let cancelled = || cancelled.load(Ordering::SeqCst) || selection::revoked(job, &authorization);
    let context = format!("form:{}", root.display());
    let reply = app
        .state::<session::Runtime>()
        .run(
            app,
            addon,
            &request.command,
            Invocation::Form {
                surface,
                values: request.form,
                geometry: geometry.clone(),
                anchor: request.anchor,
            },
            Some(&context),
            cancelled,
        )
        .map_err(|error| error.code())?
        .map_err(|error| selection::command_problem(error.code))?;
    let pages = batch::prepare(&root, &geometry, reply, generator, cancelled)?;
    if cancelled() {
        return Err("addon-cancelled".into());
    }
    let _idle = job.lock_idle().ok_or("addon-unavailable")?;
    if !selection::authorized(app, addon, &_idle).is_ok_and(|active| active == authorization)
        || !workspace::ensure_allowed(&roots, &request.root).is_ok_and(|resolved| resolved == root)
    {
        return Err("addon-unavailable".into());
    }
    let id = crate::helper::random_hex(16)?;
    let state = app.state::<PendingForms>();
    let mut pending = state.0.lock().unwrap();
    if pending.len() >= 20 {
        return Err("addon-too-many-proposals".into());
    }
    pending.insert(
        id.clone(),
        PendingForm {
            root,
            page_id: request.page_id,
            base,
            addon_id: request.addon_id,
            authorization,
            direct,
            generator,
            geometry: geometry.clone(),
            pages: pages.clone(),
        },
    );
    Ok(FormPreview {
        id,
        direct,
        geometry,
        pages,
    })
}

#[tauri::command]
pub(crate) fn addon_form_dismiss(state: tauri::State<'_, PendingForms>, id: String) {
    state.0.lock().unwrap().remove(&id);
}

fn take_form(
    app: &tauri::AppHandle,
    root: &Path,
    id: &str,
    generator: bool,
    direct: bool,
    commit: impl FnOnce(PendingForm) -> Result<(), storage::StorageError>,
) -> Result<bool, storage::StorageError> {
    let error = || storage::StorageError::InvalidNotebook("addon-unavailable".into());
    let state = app.state::<PendingForms>();
    let addon_id = state
        .0
        .lock()
        .unwrap()
        .get(id)
        .ok_or_else(error)?
        .addon_id
        .clone();
    let importers = app.state::<Importers>();
    let (job, addon) = importers.get(&addon_id).map_err(|_| error())?;
    let _idle = job.lock_idle().ok_or_else(error)?;
    let mut pending = state.0.lock().unwrap();
    let form = pending.get(id).ok_or_else(error)?;
    if form.generator != generator
        || form.root != root
        || !selection::authorized(app, addon, &_idle)
            .is_ok_and(|active| active == form.authorization)
    {
        return Err(error());
    }
    if !commit_allowed(direct, form.direct, || manager::direct_apply(app, addon.id)) {
        return Ok(false);
    }
    let form = pending.remove(id).ok_or_else(error)?;
    drop(pending);
    commit(form)?;
    Ok(true)
}

pub(super) fn commit_allowed(
    direct: bool,
    direct_at_start: bool,
    allowed_now: impl FnOnce() -> bool,
) -> bool {
    !direct || (direct_at_start && allowed_now())
}

#[tauri::command]
pub(crate) async fn addon_form_insert(
    app: tauri::AppHandle,
    root: String,
    page_id: String,
    id: String,
    direct: bool,
) -> Result<Option<storage::HistoryResult>, NotebookError> {
    notebook::on_page(&app.state(), &app.state(), root, page_id, {
        let app = app.clone();
        move |root, page, history| {
            let mut result = None;
            take_form(&app, root, &id, false, direct, |mut form| {
                if page != form.page_id {
                    return Err(storage::StorageError::InvalidNotebook(
                        "addon-unavailable".into(),
                    ));
                }
                let mut snapshot = storage::open_page(root, page)?;
                form.pages
                    .remove(0)
                    .insert(&mut snapshot)
                    .map_err(storage::StorageError::InvalidNotebook)?;
                result = Some(storage::commit_notebook_with_precondition(
                    root,
                    history,
                    snapshot,
                    Some(storage::ProposalPrecondition {
                        base: form.base,
                        require_retained_history: true,
                    }),
                )?);
                Ok(())
            })?;
            Ok(result)
        }
    })
    .await
}

#[tauri::command]
pub(crate) async fn addon_form_add_pages(
    app: tauri::AppHandle,
    root: String,
    id: String,
    position: storage::PagePosition,
) -> Result<storage::StructureHistoryResult, NotebookError> {
    notebook::on_notebook(&app.state(), &app.state(), root, {
        let app = app.clone();
        move |root, store| {
            let mut snapshot = None;
            take_form(&app, root, &id, true, false, |form| {
                notebook::observe_structure(store, root, &form.page_id)
                    .map_err(|error| storage::StorageError::InvalidNotebook(error.to_string()))?;
                let pages = form
                    .pages
                    .into_iter()
                    .map(PreviewPage::generated)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage::StorageError::InvalidNotebook)?;
                snapshot = Some(storage::add_generated_pages(
                    root,
                    &crate::helper::now().to_string(),
                    &position,
                    &form.geometry,
                    pages,
                )?);
                Ok(())
            })?;
            notebook::advance_structure_result(
                store,
                root,
                snapshot.ok_or_else(|| NotebookError::other("addon-unavailable"))?,
            )
        }
    })
    .await
}

#[cfg(test)]
pub(super) fn fixture() -> super::manifest::Manifest {
    let addon = Addon {
        id: "fixture",
        name: "goodtype-fixture",
        contribution_surfaces: &[Surface::InsertForm, Surface::PageGenerator, Surface::Export],
        kind: Kind::Recognize {
            recognizes: "fixture",
            check_failure: "fixture check failed",
        },
        ..catalog::MATH
    };
    manifest::parse(include_bytes!("contribution-fixture.json"), &addon, "0.1.0").unwrap()
}
