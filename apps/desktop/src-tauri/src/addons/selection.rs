use super::{session, *};
use crate::helper::proposals::ProposalItem;
use goodtype_core::{
    ObjectFields, PageGeometry, PageObject,
    storage::{self, NotebookSnapshot, ProposalBase, StorageError},
};
use goodtype_typst::{CompileRequest, GrantedFiles};
use serde_json::{Value, json};

const MAX_SOURCE: usize = 64 * 1024;
const MAX_PENDING: usize = 20;
/// Total budget for compiling a returned batch after the addon replied.
const COMPILE_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Name {
    en: String,
    de: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectionCommand {
    addon_id: String,
    command: String,
    label: Name,
}

#[tauri::command]
pub(crate) async fn addon_selection_commands(
    app: tauri::AppHandle,
) -> Result<Vec<SelectionCommand>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Importers>();
        let mut result = Vec::new();
        for &addon in CATALOG {
            let (job, _) = state.get(addon.id)?;
            let Ok(release) = authorization(&app, job, addon) else {
                continue;
            };
            let Ok(manifest) = manager::installed(addon, &release.path, &release.version, false)
            else {
                continue;
            };
            result.extend(
                manifest
                    .selection_labels()
                    .map(|(command, en, de)| SelectionCommand {
                        addon_id: addon.id.into(),
                        command: command.into(),
                        label: Name {
                            en: en.into(),
                            de: de.into(),
                        },
                    }),
            );
        }
        Ok(result)
    })
    .await
    .map_err(|_| "addon-unavailable")?
}

#[derive(Clone, PartialEq)]
pub(super) struct Authorization {
    pub(super) version: String,
    pub(super) path: PathBuf,
    epoch: u64,
}

pub(super) fn authorization(
    app: &tauri::AppHandle,
    job: &Importer,
    addon: &'static Addon,
) -> Result<Authorization, String> {
    let state = job.lock_idle().ok_or("addon-unavailable")?;
    authorized(app, addon, &state)
}

pub(super) fn authorized(
    app: &tauri::AppHandle,
    addon: &'static Addon,
    state: &State,
) -> Result<Authorization, String> {
    if !platform_supported() || !manager::enabled(app, addon)? {
        return Err("addon-unavailable".into());
    }
    let (version, path) = install_dir(app, addon)?
        .active()
        .ok_or("addon-unavailable")?;
    if state.incompatible_session.as_ref() == Some(&version) {
        return Err("addon-needs-update".into());
    }
    Ok(Authorization {
        version,
        path,
        epoch: state.authorization_epoch,
    })
}

/// In memory only, so a run can poll it every 20 ms: disable, remove, update and rollback reserve
/// the job and bump the epoch. Policy and install are read from disk at invocation and commit.
pub(super) fn revoked(job: &Importer, original: &Authorization) -> bool {
    let state = job.state.lock().unwrap();
    state.busy() || state.authorization_epoch != original.epoch
}

pub(crate) struct Capture {
    pub root: PathBuf,
    base: ProposalBase,
    geometry: PageGeometry,
    ids: Vec<String>,
    ink_hash: [u8; 32],
    scope: Value,
    anchor: Value,
}

fn invalid(message: &str) -> StorageError {
    StorageError::InvalidNotebook(message.into())
}

pub(crate) fn scope(
    geometry: &PageGeometry,
    strokes: &crate::recognition::math_process::MathSelection,
) -> Value {
    let strokes: Vec<_> = strokes.strokes.iter().enumerate().map(|(index, stroke)|
        json!({"id":format!("s{}", index + 1),"widthPt":stroke.width_pt,"points":stroke.points})
    ).collect();
    json!({"kind":"ink-selection","page":geometry,"strokes":strokes})
}

fn selected_ink<'a>(
    page: &'a NotebookSnapshot,
    ids: &[String],
) -> Result<Vec<&'a goodtype_core::Stroke>, StorageError> {
    let requested: HashSet<_> = ids.iter().collect();
    if page.page.calculation.is_some()
        || requested.len() != ids.len()
        || !(1..=2000).contains(&ids.len())
    {
        return Err(invalid("addon-invalid-selection"));
    }
    let strokes: Vec<_> = page
        .ink_layers
        .iter()
        .flat_map(|layer| &layer.strokes)
        .filter(|stroke| requested.contains(&stroke.id))
        .collect();
    if strokes.len() != ids.len() || strokes.iter().map(|s| s.points.len()).sum::<usize>() > 200_000
    {
        return Err(invalid("addon-invalid-selection"));
    }
    Ok(strokes)
}

fn ink_hash(strokes: &[&goodtype_core::Stroke]) -> Result<[u8; 32], StorageError> {
    Ok(Sha256::digest(serde_json::to_vec(strokes)?).into())
}

pub(crate) fn capture(
    root: &Path,
    page_id: &str,
    ids: Vec<String>,
) -> Result<Capture, StorageError> {
    use crate::recognition::math_process::{MathSelection, MathStroke};
    let snapshot = storage::open_page(root, page_id)?;
    let strokes = selected_ink(&snapshot, &ids)?;
    let ink_hash = ink_hash(&strokes)?;
    let selection = MathSelection {
        strokes: strokes
            .into_iter()
            .map(|s| {
                let t = &s.transform;
                let (sin, cos) = t.rotation.to_radians().sin_cos();
                MathStroke {
                    width_pt: s.width_pt * t.scale_x.abs(),
                    points: s
                        .points
                        .iter()
                        .map(|p| {
                            let x = p.x * t.scale_x;
                            let y = p.y * t.scale_y;
                            [
                                x * cos - y * sin + t.translate_x,
                                x * sin + y * cos + t.translate_y,
                            ]
                        })
                        .collect(),
                }
            })
            .collect(),
    };
    selection
        .validate()
        .map_err(|_| invalid("addon-invalid-selection"))?;
    let (mut x, mut y, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &[px, py] in selection.strokes.iter().flat_map(|stroke| &stroke.points) {
        x = x.min(px);
        y = y.min(py);
        right = right.max(px);
        bottom = bottom.max(py);
    }
    let geometry = snapshot.page.geometry.clone();
    let scope = scope(&geometry, &selection);
    if serde_json::to_vec(&scope)?.len() > session::MAX_SCOPE {
        return Err(invalid("addon-invalid-selection"));
    }
    Ok(Capture {
        root: root.into(),
        base: storage::proposal_base(root, &snapshot, &[])?,
        geometry,
        ids,
        ink_hash,
        scope,
        anchor: json!({"x":x,"y":y,"width":(right-x).max(1.0),"height":(bottom-y).max(1.0)}),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultBody {
    batch: Batch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    operations: Vec<Operation>,
}

#[derive(Clone, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum Operation {
    AddTypstBlock {
        source: String,
        x_pt: f64,
        y_pt: f64,
        layout_width_pt: f64,
    },
}

pub(super) fn operations(reply: Value, geometry: &PageGeometry) -> Result<Vec<Operation>, String> {
    if !reply["batch"]["operations"]
        .as_array()
        .is_some_and(|ops| (1..=32).contains(&ops.len()) && ops.iter().all(Value::is_object))
    {
        return Err("addon-invalid-response".into());
    }
    let body: ResultBody = serde_json::from_value(reply).map_err(|_| "addon-invalid-response")?;
    for op in &body.batch.operations {
        let Operation::AddTypstBlock {
            source,
            x_pt,
            y_pt,
            layout_width_pt,
        } = op;
        if source.trim().is_empty()
            || source.len() > MAX_SOURCE
            || !x_pt.is_finite()
            || !y_pt.is_finite()
            || !layout_width_pt.is_finite()
            || *x_pt < 0.0
            || *y_pt < 0.0
            || *y_pt >= geometry.height_pt
            || *layout_width_pt <= 0.0
            || x_pt + layout_width_pt > geometry.width_pt
        {
            return Err("addon-invalid-response".into());
        }
    }
    Ok(body.batch.operations)
}

/// Validates and compiles a whole batch; one invalid operation rejects all of it. Up to 32
/// compiles share one deadline, and a cancel or revocation is honored between them.
pub(super) fn items(
    reply: Value,
    capture: &Capture,
    id: &str,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<ProposalItem>, String> {
    let deadline = std::time::Instant::now() + COMPILE_BUDGET;
    operations(reply, &capture.geometry)?
        .into_iter()
        .enumerate()
        .map(|(index, op)| {
            if cancelled() {
                return Err("addon-cancelled".into());
            }
            if std::time::Instant::now() >= deadline {
                return Err("addon-timeout".into());
            }
            compile(
                &capture.root,
                &capture.geometry,
                format!("addon-{id}-{index}"),
                op,
            )
        })
        .collect()
}

pub(super) fn compile(
    root: &Path,
    geometry: &PageGeometry,
    id: String,
    op: Operation,
) -> Result<ProposalItem, String> {
    let Operation::AddTypstBlock {
        source,
        x_pt,
        y_pt,
        layout_width_pt,
    } = op;
    if source.trim().is_empty() || source.len() > MAX_SOURCE {
        return Err("addon-invalid-response".into());
    }
    let result = goodtype_typst::compile_block_granted(
        root,
        &CompileRequest {
            source: source.clone(),
            width_pt: layout_width_pt,
            generation: 0,
            allow_remote_packages: false,
        },
        &GrantedFiles::new(),
    )
    .map_err(|_| "addon-compile-failed")?;
    let svg = result.svg.ok_or("addon-compile-failed")?;
    let (w, h) = (
        result.width_pt.unwrap_or(layout_width_pt),
        result.height_pt.unwrap_or(1.0),
    );
    if !w.is_finite()
        || !h.is_finite()
        || w <= 0.0
        || h <= 0.0
        || x_pt + w > geometry.width_pt
        || y_pt + h > geometry.height_pt
    {
        return Err("addon-does-not-fit".into());
    }
    let timestamp = crate::helper::now().to_string();
    let object = PageObject::Typst {
        source_path: format!("blocks/{id}.typ"),
        fields: ObjectFields {
            id,
            x: x_pt,
            y: y_pt,
            rotation: 0.0,
            scale: 1.0,
            z_index: 0,
            reading_order: 0,
            group_id: None,
            created_at: timestamp.clone(),
            modified_at: timestamp,
            addon_data: None,
            ports: None,
        },
        layout_width_pt,
        measured_width_pt: w,
        measured_height_pt: h,
    };
    let hash = format!("{:x}", Sha256::digest(source.as_bytes()));
    Ok(ProposalItem {
        object,
        source,
        hash,
        svg,
        before_source: None,
        before_svg: None,
        width_pt: w,
        height_pt: h,
        placement: json!({"x":x_pt,"y":y_pt}),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Review {
    id: String,
    version: u64,
    root: String,
    pub(crate) page_id: String,
    status: &'static str,
    pub(crate) addon: Name,
    items: Vec<ProposalItem>,
    anchor: Value,
    latex: Option<String>,
    kind: &'static str,
}

pub(super) struct Pending {
    review: Review,
    capture: Capture,
    addon: &'static Addon,
    authorization: Authorization,
    direct: bool,
    correction: Option<Operation>,
}

#[derive(Default)]
pub(crate) struct Proposals {
    pending: Mutex<HashMap<String, Pending>>,
    running: Mutex<Option<(String, Arc<AtomicBool>)>>,
}

/// `cancellation` comes from [`Proposals::begin`], registered before the capture.
pub(crate) fn invoke(
    app: &tauri::AppHandle,
    addon_id: &str,
    command: &str,
    capture: Capture,
    cancellation: Arc<AtomicBool>,
    review_first: bool,
) -> Result<(Review, bool), String> {
    let importers = app.state::<Importers>();
    let (job, addon) = importers.get(addon_id)?;
    let original = authorization(app, job, addon)?;
    let manifest = manager::installed(addon, &original.path, &original.version, true)
        .map_err(manager::manifest_problem)?;
    if !manifest.supports_selection(command) {
        return Err("addon-unavailable".into());
    }
    let name = Name {
        en: manifest.strings.en["name"].clone(),
        de: manifest.strings.de["name"].clone(),
    };
    let direct = !review_first && manager::direct_apply(app, addon.id);
    let proposals = app.state::<Proposals>();
    {
        let context = warm_context("notebook", &capture.root);
        let cancelled = || cancellation.load(Ordering::SeqCst) || revoked(job, &original);
        let reply = app
            .state::<session::Runtime>()
            .run(
                app,
                addon,
                command,
                super::contributions::Invocation::Selection(capture.scope.clone()),
                Some(&context),
                cancelled,
            )
            .map_err(|failure| failure.code())?;
        let id = crate::helper::random_hex(16)?;
        let (items, latex, correction) = match reply {
            Ok(reply) => (items(reply, &capture, &id, cancelled)?, None, None),
            Err(error) if error.code == -32003 && addon.id == "math" => {
                let latex = unsupported_latex(error)?;
                let x_pt = capture.anchor["x"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .clamp(0.0, (capture.geometry.width_pt - 120.0).max(0.0));
                let y_pt = capture.anchor["y"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .clamp(0.0, (capture.geometry.height_pt - 48.0).max(0.0));
                let layout_width_pt = 120.0_f64.min(capture.geometry.width_pt);
                (
                    vec![],
                    Some(latex),
                    Some(Operation::AddTypstBlock {
                        source: String::new(),
                        x_pt,
                        y_pt,
                        layout_width_pt,
                    }),
                )
            }
            Err(error) => return Err(command_problem(error.code).into()),
        };
        if cancelled() {
            return Err("addon-cancelled".into());
        }
        let direct = direct && latex.is_none();
        let review = Review {
            id: id.clone(),
            version: 1,
            root: capture.root.to_string_lossy().into_owned(),
            page_id: capture.base.page_id.clone(),
            status: "pending",
            addon: name,
            items,
            anchor: capture.anchor.clone(),
            latex,
            kind: "blocks",
        };
        let mut pending = proposals.pending.lock().unwrap();
        if pending.len() >= MAX_PENDING {
            return Err("addon-too-many-proposals".into());
        }
        pending.insert(
            id,
            Pending {
                review: review.clone(),
                capture,
                addon,
                authorization: original,
                direct,
                correction,
            },
        );
        Ok((review, direct))
    }
}

/// A warm worker serves one notebook and one kind of grant. The fingerprint in the runtime adds
/// the verified version and inventory; a disable, remove or update ends the worker outright.
fn warm_context(grant: &str, root: &Path) -> String {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.into());
    format!("{grant}:{}", root.display())
}

/// The active install's own manifest decides the transport, never whether it is authorized: a
/// session install that is disabled, revoked or failed its handshake must not fall through to
/// the unconfined legacy `--serve` process.
pub(crate) fn is_session_math(app: &tauri::AppHandle) -> bool {
    let addon = &super::catalog::MATH;
    install_dir(app, addon)
        .ok()
        .and_then(|dir| dir.active())
        .and_then(|(version, path)| manager::installed(addon, &path, &version, false).ok())
        .is_some_and(|manifest| manifest.uses_session())
}

/// `None` keeps the legacy route; a session transport yields its authorization or its refusal.
pub(super) fn session_route(
    session: bool,
    authorize: impl FnOnce() -> Result<Authorization, String>,
) -> Result<Option<Authorization>, String> {
    if session {
        authorize().map(Some)
    } else {
        Ok(None)
    }
}

/// Read-only recognition text from a session math install: the transcription panel reviews it
/// and the AI bridge returns it, so the batch's mutation candidate is discarded here. `None`
/// means the active install is not a session addon and the legacy path applies.
pub(crate) fn session_math(
    app: &tauri::AppHandle,
    selection: &crate::recognition::math_process::MathSelection,
    page: &PageGeometry,
    grant: &str,
    root: &Path,
) -> Result<Option<Result<String, String>>, String> {
    let addon = &super::catalog::MATH;
    let importers = app.state::<Importers>();
    let (job, _) = importers.get(addon.id)?;
    let Some(original) = session_route(is_session_math(app), || authorization(app, job, addon))?
    else {
        return Ok(None);
    };
    if !(page.width_pt.is_finite() && page.height_pt.is_finite())
        || page.width_pt <= 0.0
        || page.height_pt <= 0.0
    {
        return Err("addon-invalid-selection".into());
    }
    let scope = scope(page, selection);
    if serde_json::to_vec(&scope).map_err(|e| e.to_string())?.len() > session::MAX_SCOPE {
        return Err("addon-invalid-selection".into());
    }
    let context = warm_context(grant, root);
    let cancelled = || revoked(job, &original);
    let reply = app
        .state::<session::Runtime>()
        .run(
            app,
            addon,
            "recognize-math",
            super::contributions::Invocation::Selection(scope),
            Some(&context),
            cancelled,
        )
        .map_err(|failure| failure.code())?;
    Ok(Some(match reply {
        Ok(reply) => {
            let operations = operations(reply, page)?;
            let [Operation::AddTypstBlock { source, .. }] = operations.as_slice() else {
                return Err("addon-invalid-response".into());
            };
            let trimmed = source.trim();
            Ok(trimmed
                .strip_prefix('$')
                .and_then(|inner| inner.strip_suffix('$'))
                .unwrap_or(trimmed)
                .trim()
                .to_owned())
        }
        Err(error) if error.code == -32003 => Err(unsupported_latex(error)?),
        Err(error) => return Err(command_problem(error.code).into()),
    }))
}

pub(super) fn unsupported_latex(error: session::CommandError) -> Result<String, String> {
    error
        .data
        .as_ref()
        .and_then(Value::as_object)
        .filter(|data| data.len() == 1)
        .and_then(|data| data.get("latex"))
        .and_then(Value::as_str)
        .filter(|latex| latex.len() <= MAX_SOURCE)
        .map(str::to_owned)
        .ok_or_else(|| "addon-invalid-response".into())
}

pub(super) fn command_problem(code: i32) -> &'static str {
    match code {
        -32002 => "addon-cancelled",
        -32003 => "addon-unsupported",
        _ => "addon-command-failed",
    }
}

impl Proposals {
    /// Registers the run before its capture, so a cancel that arrives during capture or
    /// verification is latched. Pair with [`Proposals::finish`].
    pub(crate) fn begin(&self, request_id: &str) -> Result<Arc<AtomicBool>, String> {
        let mut running = self.running.lock().unwrap();
        if running.is_some() || request_id.is_empty() || request_id.len() > 80 {
            return Err("addon-unavailable".into());
        }
        let flag = Arc::new(AtomicBool::new(false));
        *running = Some((request_id.into(), Arc::clone(&flag)));
        Ok(flag)
    }

    pub(super) fn cancel(&self, request_id: &str) {
        if let Some((id, flag)) = self.running.lock().unwrap().as_ref()
            && id == request_id
        {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub(crate) fn finish(&self, request_id: &str) {
        let mut running = self.running.lock().unwrap();
        if running.as_ref().is_some_and(|(id, _)| id == request_id) {
            running.take();
        }
    }

    /// A disable, removal, update or rollback (an epoch bump) can never commit these again.
    pub(super) fn forget_addon(&self, addon_id: &str) {
        self.pending
            .lock()
            .unwrap()
            .retain(|_, p| p.addon.id != addon_id);
    }

    /// Closing a notebook drops its proposals; reopening requires a new invocation.
    pub(crate) fn forget_root(&self, root: &Path) {
        let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.into());
        let root = canonical(root);
        self.pending
            .lock()
            .unwrap()
            .retain(|_, p| canonical(&p.capture.root) != root);
    }
}

#[tauri::command]
pub(crate) fn addon_cancel(proposals: tauri::State<'_, Proposals>, request_id: String) {
    proposals.cancel(&request_id);
}

#[tauri::command]
pub(crate) fn addon_dismiss(proposals: tauri::State<'_, Proposals>, id: String) {
    proposals.pending.lock().unwrap().remove(&id);
}

#[tauri::command]
pub(crate) async fn addon_edit(
    app: tauri::AppHandle,
    id: String,
    version: u64,
    sources: Vec<String>,
) -> Result<Review, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Proposals>().edit(&id, version, sources)
    })
    .await
    .map_err(|_| "addon-compile-failed")?
}

impl Proposals {
    /// A reviewed correction replaces each source, revalidates it as a fresh candidate and bumps
    /// the version; an edited proposal is never applied without review.
    pub(super) fn edit(
        &self,
        id: &str,
        version: u64,
        sources: Vec<String>,
    ) -> Result<Review, String> {
        let (root, geometry, revised) = {
            let pending = self.pending.lock().unwrap();
            let p = pending
                .get(id)
                .filter(|p| p.review.version == version)
                .ok_or("proposal_changed")?;
            let mut ops: Vec<_> = p
                .review
                .items
                .iter()
                .filter_map(|item| match &item.object {
                    PageObject::Typst {
                        fields,
                        layout_width_pt,
                        ..
                    } => Some(Operation::AddTypstBlock {
                        source: String::new(),
                        x_pt: fields.x,
                        y_pt: fields.y,
                        layout_width_pt: *layout_width_pt,
                    }),
                    _ => None,
                })
                .collect();
            ops.extend(p.correction.clone());
            if ops.len() != sources.len()
                || ops.len() != p.review.items.len() + usize::from(p.correction.is_some())
            {
                return Err("addon-invalid-response".into());
            }
            for (op, replacement) in ops.iter_mut().zip(sources) {
                let Operation::AddTypstBlock { source, .. } = op;
                *source = replacement;
            }
            (p.capture.root.clone(), p.capture.geometry.clone(), ops)
        };
        let items = revised
            .into_iter()
            .enumerate()
            .map(|(index, op)| compile(&root, &geometry, format!("addon-{id}-{index}"), op))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pending = self.pending.lock().unwrap();
        let p = pending
            .get_mut(id)
            .filter(|p| p.review.version == version)
            .ok_or("proposal_changed")?;
        p.review.items = items;
        p.review.version += 1;
        p.review.latex = None;
        p.correction = None;
        p.direct = false;
        Ok(p.review.clone())
    }
}

pub(crate) fn insert(
    app: &tauri::AppHandle,
    root: &Path,
    page_id: &str,
    history: &mut storage::NotebookHistory,
    id: &str,
    version: u64,
    direct: bool,
) -> Result<Option<storage::HistoryResult>, StorageError> {
    let importers = app.state::<Importers>();
    app.state::<Proposals>()
        .commit(root, page_id, history, id, version, |p| {
            let (job, _) = importers
                .get(p.addon.id)
                .map_err(|_| invalid("addon-unavailable"))?;
            // Holding the idle job keeps disable, remove and update out until the commit ends.
            let state = job
                .lock_idle()
                .ok_or_else(|| invalid("addon-unavailable"))?;
            if !authorized(app, p.addon, &state).is_ok_and(|current| current == p.authorization) {
                return Err(invalid("addon-unavailable"));
            }
            let allowed = !direct || (p.direct && manager::direct_apply(app, p.addon.id));
            Ok(allowed.then_some(state))
        })
}

impl Proposals {
    /// Commits a reviewed or direct proposal as one ordinary page transaction. `authorize`
    /// rechecks the addon under the notebook lock; `None` declines a direct apply so the result
    /// stays a proposal. The returned guard is held until the transaction ends.
    pub(super) fn commit<G>(
        &self,
        root: &Path,
        page_id: &str,
        history: &mut storage::NotebookHistory,
        id: &str,
        version: u64,
        authorize: impl FnOnce(&Pending) -> Result<Option<G>, StorageError>,
    ) -> Result<Option<storage::HistoryResult>, StorageError> {
        let mut pending = self.pending.lock().unwrap();
        let p = pending.get(id).ok_or_else(|| invalid("proposal_changed"))?;
        if p.capture.root != root
            || p.review.page_id != page_id
            || p.review.version != version
            || p.review.items.is_empty()
        {
            return Err(invalid("proposal_changed"));
        }
        let Some(_guard) = authorize(p)? else {
            return Ok(None);
        };
        let mut candidate = storage::open_page(root, page_id)?;
        let stale = StorageError::ProposalStale(vec![storage::ChangedDependency::Page {
            change: storage::DependencyChange::Modified,
        }]);
        let unchanged = candidate.page.revision == p.capture.base.revision
            && selected_ink(&candidate, &p.capture.ids)
                .ok()
                .and_then(|strokes| ink_hash(&strokes).ok())
                == Some(p.capture.ink_hash);
        if !unchanged {
            // A stale capture can never become valid again; the user invokes the command anew.
            pending.remove(id);
            return Err(stale);
        }
        let mut z = candidate
            .page
            .objects
            .iter()
            .map(|o| o.fields().z_index)
            .chain(
                candidate
                    .ink_layers
                    .iter()
                    .flat_map(|l| l.strokes.iter().map(|s| s.z_index)),
            )
            .max()
            .unwrap_or(0);
        for item in &p.review.items {
            let mut object = item.object.clone();
            let path = object
                .searchable_source()
                .ok_or_else(|| invalid("addon-invalid-response"))?
                .to_owned();
            let fields = object.fields_mut();
            z = z
                .checked_add(1)
                .ok_or_else(|| invalid("addon-invalid-response"))?;
            fields.z_index = z;
            fields.reading_order = candidate
                .page
                .reading_order
                .len()
                .try_into()
                .map_err(|_| invalid("addon-invalid-response"))?;
            candidate.page.reading_order.push(fields.id.clone());
            candidate.page.objects.push(object);
            candidate.blocks.push(storage::StoredFile {
                path,
                bytes: item.source.as_bytes().to_vec(),
            });
        }
        let result = storage::commit_notebook_with_precondition(
            root,
            history,
            candidate,
            Some(storage::ProposalPrecondition {
                base: p.capture.base.clone(),
                require_retained_history: true,
            }),
        )?;
        pending.remove(id);
        Ok(Some(result))
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
