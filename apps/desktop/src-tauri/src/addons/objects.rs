//! Capture only the canonical objects admitted by an explicit addon action.
use super::{Importers, batch, contributions::Invocation, selection, session};
use crate::{
    notebook::{self, NotebookError, NotebookHistories},
    workspace::{self, AllowedRoots},
};
use goodtype_core::{PageObject, ShapePoint, addon_data, storage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use tauri::Manager;

#[path = "scoped_reads.rs"]
pub(super) mod scoped_reads;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    root: String,
    page_id: String,
    revision: u64,
    addon_id: String,
    action: Action,
    request_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Action {
    Tool {
        id: String,
        points: Vec<Point>,
        #[serde(default)]
        pen: Option<Pen>,
        #[serde(default)]
        style: Option<goodtype_core::ShapeStyle>,
    },
    PathEdit {
        id: String,
        object_id: String,
        edit: Edit,
    },
    Command {
        id: String,
        selected: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Edit {
    Endpoint {
        end: End,
        point: ShapePoint,
        attach: Option<Attachment>,
    },
    Choice {
        value: String,
    },
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum End {
    Start,
    End,
}

/// The active pen, so a tool's new objects can match it (the addon decides whether to use it).
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pen {
    stroke_color: String,
    stroke_width_pt: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Point {
    x: f64,
    y: f64,
    attach: Option<Attachment>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Attachment {
    object_id: String,
    port: usize,
}

fn invalid(message: &str) -> storage::StorageError {
    storage::StorageError::InvalidNotebook(message.into())
}

fn geometry_value(page: &goodtype_core::Page) -> Value {
    json!({"id": page.id, "widthPt": page.geometry.width_pt, "heightPt": page.geometry.height_pt})
}

fn transformed_extent(object: &PageObject, bundle: &storage::NotebookSnapshot) -> [f64; 4] {
    let page = &bundle.page;
    if let PageObject::InkGroup {
        ink_layer_id,
        stroke_ids,
        ..
    } = object
    {
        let ids: HashSet<_> = stroke_ids.iter().map(String::as_str).collect();
        if let Some(bounds) = goodtype_core::outline::stroke_bounds(
            bundle
                .ink_layers
                .iter()
                .filter(|layer| layer.id == *ink_layer_id)
                .flat_map(|layer| &layer.strokes)
                .filter(|stroke| ids.contains(stroke.id.as_str())),
        ) {
            return bounds;
        }
    }
    let extent = match object {
        PageObject::Shape { geometry, .. } => batch::shape_extent(geometry),
        PageObject::Typst {
            measured_width_pt,
            measured_height_pt,
            ..
        } => Some([0.0, 0.0, *measured_width_pt, *measured_height_pt]),
        PageObject::Image {
            width_pt,
            height_pt,
            ..
        } => Some([0.0, 0.0, *width_pt, *height_pt]),
        PageObject::PdfMaterial {
            source_width_pt,
            source_height_pt,
            ..
        } => Some([0.0, 0.0, *source_width_pt, *source_height_pt]),
        PageObject::PageTypst { .. } => {
            Some([0.0, 0.0, page.geometry.width_pt, page.geometry.height_pt])
        }
        _ => None,
    }
    .unwrap_or([0.0; 4]);
    let [left, top, right, bottom] = extent;
    [(left, top), (left, bottom), (right, top), (right, bottom)]
        .map(|(x, y)| addon_data::to_page_point(object.fields(), ShapePoint { x, y }))
        .into_iter()
        .fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |[left, top, right, bottom], point| {
                [
                    left.min(point.x),
                    top.min(point.y),
                    right.max(point.x),
                    bottom.max(point.y),
                ]
            },
        )
}

fn object_extent(object: &PageObject, bundle: &storage::NotebookSnapshot) -> [f64; 4] {
    let page = &bundle.page;
    let PageObject::Group { child_ids, .. } = object else {
        return transformed_extent(object, bundle);
    };
    // Group children already live in page coordinates. The group transform positions its ports.
    let index: std::collections::HashMap<_, _> = page
        .objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect();
    let mut pending: Vec<_> = child_ids.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut bounds: Option<[f64; 4]> = None;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(child) = index.get(id) else {
            continue;
        };
        if let PageObject::Group { child_ids, .. } = child {
            pending.extend(child_ids.iter().map(String::as_str));
        } else {
            let [left, top, right, bottom] = transformed_extent(child, bundle);
            bounds = Some(bounds.map_or([left, top, right, bottom], |[l, t, r, b]| {
                [l.min(left), t.min(top), r.max(right), b.max(bottom)]
            }));
        }
    }
    bounds.unwrap_or_else(|| transformed_extent(object, bundle))
}

fn object_snapshot(object: &PageObject, bundle: &storage::NotebookSnapshot) -> Value {
    let fields = object.fields();
    let [x, y, right, bottom] = object_extent(object, bundle);
    let kind = match object {
        PageObject::Typst { .. } => "typst",
        PageObject::PageTypst { .. } => "page_typst",
        PageObject::Image { .. } => "image",
        PageObject::PdfMaterial { .. } => "pdf_material",
        PageObject::InkGroup { .. } => "ink_group",
        PageObject::Shape { .. } => "shape",
        PageObject::Group { .. } => "group",
    };
    let mut value = json!({
        "id": fields.id, "type": kind, "x": fields.x, "y": fields.y,
        "rotation": fields.rotation, "scale": fields.scale,
        "bounds": {"x": x, "y": y, "width": right - x, "height": bottom - y},
        "ports": addon_data::page_ports(object)
    });
    if let PageObject::Shape { geometry, .. } = object {
        value["geometry"] = json!(geometry);
    }
    value
}

fn own_value(object: &PageObject, addon: &str) -> Value {
    let mut value = json!(object);
    if let Some(entries) = value.get_mut("addonData").and_then(Value::as_object_mut) {
        entries.retain(|id, _| id == addon);
    }
    value
}

fn port_target<'a>(
    page: &'a goodtype_core::Page,
    source: Option<&PageObject>,
    target: &str,
) -> Option<&'a PageObject> {
    let find = |id: &str| page.objects.iter().find(|object| object.fields().id == id);
    let target = find(target)?;
    let fields = target.fields();
    if source.is_some_and(|source| source.fields().id == fields.id)
        || (matches!(target, PageObject::Shape { geometry, .. } if batch::path_ends(geometry).is_some())
            && fields
                .addon_data
                .as_ref()
                .is_some_and(|entries| !entries.is_empty()))
    {
        return None;
    }
    let ancestors = |object: &PageObject| {
        let mut parents = Vec::new();
        let mut parent = object.fields().group_id.as_deref();
        while let Some(group) = parent.and_then(find) {
            parents.push(group);
            parent = group.fields().group_id.as_deref();
        }
        parents
    };
    if source.is_some_and(|source| {
        ancestors(source)
            .iter()
            .any(|group| group.fields().id == fields.id)
    }) || ancestors(target).iter().any(|group| {
        source.is_some_and(|source| group.fields().id == source.fields().id)
            || group.fields().ports.is_some()
    }) {
        return None;
    }
    Some(target)
}

pub(super) fn capture_path(
    bundle: &storage::NotebookSnapshot,
    addon: &str,
    id: String,
    edit: Edit,
) -> Result<(Invocation, batch::PathGrant, bool), String> {
    let page = &bundle.page;
    let Some(
        source @ PageObject::Shape {
            fields,
            geometry,
            style,
        },
    ) = page.objects.iter().find(|object| object.fields().id == id)
    else {
        return Err(batch::FORBIDDEN.into());
    };
    let entry = fields
        .addon_data
        .as_ref()
        .and_then(|entries| entries.get(addon));
    if entry.is_none()
        && (fields
            .addon_data
            .as_ref()
            .is_some_and(|entries| !entries.is_empty())
            || (style.start_cap.is_none() && style.end_cap.is_none()))
    {
        return Err(batch::FORBIDDEN.into());
    }
    let ends = batch::path_ends(geometry).ok_or(batch::FORBIDDEN)?;
    let mut points = ends.map(|point| {
        let ShapePoint { x, y } = addon_data::to_page_point(fields, point);
        Point { x, y, attach: None }
    });
    let mut refs = Vec::new();
    let mut grant = batch::PathGrant {
        id,
        refs: HashSet::new(),
        attached: None,
    };
    if let Some(entry) = entry {
        for id in &entry.refs {
            if let Some(object) = port_target(page, Some(source), id) {
                grant.refs.insert(id.clone());
                refs.push(object_snapshot(object, bundle));
            }
        }
    }
    let (edit, drop) = match edit {
        Edit::Choice { value } => (json!({"kind": "choice", "value": value}), false),
        Edit::Endpoint {
            end,
            mut point,
            attach,
        } => {
            let index = match end {
                End::Start => 0,
                End::End => 1,
            };
            if let Some(attachment) = &attach {
                let target = port_target(page, Some(source), &attachment.object_id)
                    .ok_or(batch::FORBIDDEN)?;
                point = *addon_data::page_ports(target)
                    .get(attachment.port)
                    .ok_or(batch::FORBIDDEN)?;
                if grant.refs.insert(attachment.object_id.clone()) {
                    refs.push(object_snapshot(target, bundle));
                }
                grant.attached = Some((attachment.object_id.clone(), index, point));
            } else if entry.is_none() {
                return Err(batch::FORBIDDEN.into());
            }
            if !(0.0..=page.geometry.width_pt).contains(&point.x)
                || !(0.0..=page.geometry.height_pt).contains(&point.y)
            {
                return Err("addon-does-not-fit".into());
            }
            let drop = attach.is_some();
            points[index] = Point {
                x: point.x,
                y: point.y,
                attach,
            };
            (json!({"kind": "endpoint", "end": end}), drop)
        }
    };
    let mut object = json!({"id": fields.id, "type": "shape", "x": fields.x, "y": fields.y,
        "rotation": fields.rotation, "scale": fields.scale, "geometry": geometry, "style": style});
    if let Some(entry) = entry {
        object["addonData"] = json!({addon: entry});
    }
    Ok((
        Invocation::PathEdit(json!({"page": geometry_value(page), "edit": edit,
        "object": object, "points": points, "refs": refs})),
        grant,
        drop,
    ))
}

fn capture(
    bundle: &storage::NotebookSnapshot,
    addon: &str,
    action: Action,
) -> Result<(String, Invocation, Option<HashSet<String>>), String> {
    let page = &bundle.page;
    let objects = &page.objects;
    let find = |id: &str| objects.iter().find(|obj| obj.fields().id == id);
    match action {
        Action::PathEdit { .. } => Err(batch::FORBIDDEN.into()),
        Action::Tool {
            id,
            mut points,
            pen,
            style,
        } => {
            if style
                .as_ref()
                .is_some_and(|style| style.validate().is_err())
                || (style.is_some() && pen.is_some())
            {
                return Err("addon-invalid-response".into());
            }
            if let Some(pen) = &pen
                && (!goodtype_core::valid_hex_color(&pen.stroke_color)
                    || !pen.stroke_width_pt.is_finite()
                    || !(0.05..=64.0).contains(&pen.stroke_width_pt))
            {
                return Err("addon-invalid-response".into());
            }
            if !(1..=2).contains(&points.len()) {
                return Err("addon-invalid-response".into());
            }
            let mut attached = Vec::new();
            let mut seen = HashSet::new();
            for point in &mut points {
                if let Some(attachment) = &point.attach {
                    let object = port_target(page, None, &attachment.object_id)
                        .ok_or("addon-object-permission")?;
                    let port = addon_data::page_ports(object)
                        .get(attachment.port)
                        .copied()
                        .ok_or("addon-object-permission")?;
                    point.x = port.x;
                    point.y = port.y;
                    if seen.insert(&attachment.object_id) {
                        attached.push(object_snapshot(object, bundle));
                    }
                } else if !(0.0..=page.geometry.width_pt).contains(&point.x)
                    || !(0.0..=page.geometry.height_pt).contains(&point.y)
                {
                    return Err("addon-does-not-fit".into());
                }
            }
            Ok((
                id,
                Invocation::Tool {
                    points: points.len(),
                    snapped: !attached.is_empty(),
                    params: {
                        let mut params = json!({"page":geometry_value(page), "points":points, "objects":attached});
                        if let Some(pen) = pen {
                            params["pen"] = json!(pen);
                        }
                        if let Some(style) = style {
                            params["style"] = json!(style);
                        }
                        params
                    },
                },
                None,
            ))
        }
        Action::Command { id, selected } => {
            if !(1..=64).contains(&selected.len()) {
                return Err("addon-object-permission".into());
            }
            let mut own = HashSet::new();
            let mut refs = HashSet::new();
            let mut payload = Vec::new();
            for selected_id in selected {
                let Some(object) = find(&selected_id) else {
                    continue;
                };
                let Some(entry) = object
                    .fields()
                    .addon_data
                    .as_ref()
                    .and_then(|entries| entries.get(addon))
                else {
                    continue;
                };
                if !own.insert(selected_id) {
                    continue;
                }
                payload.push(own_value(object, addon));
                refs.extend(entry.refs.iter().map(String::as_str));
            }
            if own.is_empty() {
                return Err(batch::FORBIDDEN.into());
            }
            payload.extend(
                objects
                    .iter()
                    .filter(|object| {
                        let id = &object.fields().id;
                        refs.contains(id.as_str()) && !own.contains(id)
                    })
                    .map(|object| object_snapshot(object, bundle)),
            );
            Ok((id, Invocation::OwnObjects(payload), Some(own)))
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ActionError {
    code: i32,
    message: String,
}

impl From<NotebookError> for ActionError {
    fn from(error: NotebookError) -> Self {
        Self {
            code: -32603,
            message: error.to_string(),
        }
    }
}

fn error(message: impl Into<String>) -> ActionError {
    let message = message.into();
    let code = if message == batch::FORBIDDEN {
        -32002
    } else {
        -32603
    };
    ActionError { code, message }
}

#[tauri::command]
pub(crate) async fn addon_object_action(
    app: tauri::AppHandle,
    request: Request,
) -> Result<Option<Outcome>, ActionError> {
    let request_id = request.request_id.clone();
    let proposals = app.state::<selection::Proposals>();
    let cancellation = if matches!(request.action, Action::PathEdit { .. }) {
        Some(
            proposals
                .begin(
                    request_id
                        .as_deref()
                        .ok_or_else(|| error("addon-unavailable"))?,
                )
                .map_err(error)?,
        )
    } else {
        None
    };
    let result = object_action(&app, request, cancellation.clone()).await;
    if let Some(id) = request_id.filter(|_| cancellation.is_some()) {
        proposals.finish(&id);
    }
    result
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum Outcome {
    Committed(Box<storage::HistoryResult>),
    Review {
        review: Box<selection::Review>,
        direct: bool,
    },
}

async fn object_action(
    app: &tauri::AppHandle,
    request: Request,
    cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Option<Outcome>, ActionError> {
    let handle = app.clone();
    let flag = cancellation.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let app = handle;
        let root = workspace::ensure_allowed(&app.state::<AllowedRoots>(), &request.root)
            .map_err(error)?;
        let histories = app.state::<NotebookHistories>();
        let importers = app.state::<Importers>();
        let (job, addon) = importers.get(&request.addon_id).map_err(error)?;
        let authorization = selection::authorization(&app, job, addon).map_err(error)?;
        let (snapshot, base) = notebook::with_notebook(&histories, || {
            let snapshot =
                storage::open_page(&root, &request.page_id).map_err(|e| e.to_string())?;
            if snapshot.page.revision != request.revision || snapshot.page.calculation.is_some() {
                return Err("addon-stale-page".into());
            }
            let ids = snapshot
                .page
                .objects
                .iter()
                .map(|object| object.fields().id.clone())
                .collect::<Vec<_>>();
            let base = storage::proposal_base(&root, &snapshot, &ids).map_err(|e| e.to_string())?;
            Ok((snapshot, base))
        })
        .map_err(error)?;
        let (command, invocation, scope, path) = match request.action {
            Action::PathEdit {
                id,
                object_id,
                edit,
            } => {
                let kind = match &edit {
                    Edit::Choice { .. } => "pathChoice",
                    Edit::Endpoint {
                        end: End::Start, ..
                    } => "pathStart",
                    Edit::Endpoint { end: End::End, .. } => "pathEnd",
                };
                let (invocation, grant, drop) =
                    capture_path(&snapshot, addon.id, object_id, edit).map_err(error)?;
                (id, invocation, None, Some((grant, drop, kind)))
            }
            action => {
                let (id, invocation, scope) =
                    capture(&snapshot, addon.id, action).map_err(error)?;
                (id, invocation, scope, None)
            }
        };
        let cancelled = || {
            flag.as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        };
        let direct_at_start = path.is_some() && super::manager::direct_apply(&app, addon.id);
        let context = format!("{}:{}", root.display(), request.page_id);
        let reads = scoped_reads::Reads::capture(&snapshot, &invocation);
        let reply = app
            .state::<session::Runtime>()
            .run_scoped(
                &app,
                addon,
                &command,
                (invocation, reads),
                Some(&context),
                Some(snapshot.page.revision),
                || cancelled() || selection::revoked(job, &authorization),
            )
            .map_err(|failure| error(failure.code()))?
            .map_err(|failure| ActionError {
                code: failure.code,
                message: failure.message,
            })?;
        if path.is_some() && cancelled() {
            return Err(error("addon-cancelled"));
        }
        let (candidate, review) = match path {
            Some((grant, drop, kind)) => {
                let candidate =
                    batch::apply_path_edit(snapshot, addon.id, &grant, reply).map_err(error)?;
                (Some(candidate), (!drop).then_some((grant.id, kind)))
            }
            None => (
                batch::apply_owned(&root, snapshot, addon.id, scope.as_ref(), reply)
                    .map_err(error)?,
                None,
            ),
        };
        Ok::<_, ActionError>((
            request.root,
            request.page_id,
            request.addon_id,
            candidate,
            base,
            authorization,
            review,
            direct_at_start,
        ))
    })
    .await
    .map_err(|_| error("addon-unavailable"))??;
    let (root, page, addon_id, candidate, base, authorization, review, direct_at_start) = prepared;
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let handle = app.clone();
    let cancelled = cancellation
        .as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst));
    if cancelled {
        return Err(error("addon-cancelled"));
    }
    let flag = cancellation;
    notebook::on_page(
        &app.state(),
        &app.state(),
        root,
        page,
        move |root, page, history| {
            if flag
                .as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
            {
                return Err(invalid("addon-cancelled"));
            }
            let state = handle.state::<Importers>();
            let (job, addon) = state.get(&addon_id).map_err(|e| invalid(&e))?;
            let state = job
                .lock_idle()
                .ok_or_else(|| invalid("addon-unavailable"))?;
            if !selection::authorized(&handle, addon, &state)
                .is_ok_and(|actual| actual == authorization)
            {
                return Err(invalid("addon-unavailable"));
            }
            if flag.is_some() {
                if !history.observes(page) {
                    return Err(invalid("addon-unavailable"));
                }
                let snapshot = storage::open_page(root, page)?;
                if snapshot.page.revision != base.revision {
                    return Err(invalid("addon-stale-page"));
                }
                storage::check_proposal_base(root, &snapshot, &base)?;
            }
            if let Some((id, kind)) = review {
                let (review, direct) = selection::review_path(
                    &handle,
                    (root.to_path_buf(), base),
                    candidate,
                    (&id, kind),
                    (addon, authorization, direct_at_start),
                )
                .map_err(|message| invalid(&message))?;
                return Ok(Some(Outcome::Review {
                    review: Box::new(review),
                    direct,
                }));
            }
            storage::commit_notebook_with_precondition(
                root,
                history,
                candidate,
                Some(storage::ProposalPrecondition {
                    base,
                    require_retained_history: flag.is_some(),
                }),
            )
            .map(|result| Some(Outcome::Committed(Box::new(result))))
        },
    )
    .await
    .map_err(Into::into)
}

/// Diff only canonical objects. A source-only edit can change the content of a Typst object
/// even when its geometry is unchanged.
fn reactions(
    before: &storage::NotebookSnapshot,
    after: &storage::NotebookSnapshot,
    addon: &str,
) -> Option<(Value, HashSet<String>)> {
    use std::collections::HashMap;
    let before_objects = before
        .page
        .objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect::<HashMap<_, _>>();
    let after_objects = after
        .page
        .objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect::<HashMap<_, _>>();
    let before_sources = before
        .blocks
        .iter()
        .map(|file| (file.path.as_str(), &file.bytes))
        .collect::<HashMap<_, _>>();
    let after_sources = after
        .blocks
        .iter()
        .map(|file| (file.path.as_str(), &file.bytes))
        .collect::<HashMap<_, _>>();
    let mut changed = std::collections::BTreeSet::new();
    let mut deleted = std::collections::BTreeSet::new();
    let mut own = Vec::new();
    let mut ids = HashSet::new();
    for object in &after.page.objects {
        let fields = object.fields();
        let Some(entry) = fields.addon_data.as_ref().and_then(|data| data.get(addon)) else {
            continue;
        };
        let previous_entry = before_objects
            .get(fields.id.as_str())
            .and_then(|object| object.fields().addon_data.as_ref())
            .and_then(|data| data.get(addon));
        let mut affected = false;
        for id in &entry.refs {
            let Some(old) = before_objects.get(id.as_str()) else {
                continue;
            };
            let Some(next) = after_objects.get(id.as_str()) else {
                continue;
            };
            if old != next
                || (matches!(next, PageObject::Group { .. } | PageObject::InkGroup { .. })
                    && object_extent(old, before) != object_extent(next, after))
                || next
                    .searchable_source()
                    .is_some_and(|path| before_sources.get(path) != after_sources.get(path))
            {
                changed.insert(id.as_str());
                affected = true;
            }
        }
        if let Some(previous_entry) = previous_entry {
            for id in &previous_entry.refs {
                if !after_objects.contains_key(id.as_str()) {
                    deleted.insert(id.as_str());
                    affected = true;
                }
            }
        }
        if affected {
            ids.insert(fields.id.clone());
            let mut value = own_value(object, addon);
            // Retain deleted dependencies after canonical ref cleanup, for chunk scoping only.
            value["_reactionRefs"] = json!(entry.refs);
            value["_reactionDeletedRefs"] = json!(previous_entry.map(|entry| &entry.refs));
            own.push(value);
        }
    }
    if own.is_empty() {
        return None;
    }
    let changed: Vec<_> = changed
        .into_iter()
        .map(|id| object_snapshot(after_objects[id], after))
        .collect();
    Some((
        json!({"page": geometry_value(&after.page), "changed": changed, "deleted": deleted, "own": own}),
        ids,
    ))
}

// At most 512 owned objects per addon/change; per-reply limits remain unchanged.
const REACTION_CHUNK_SIZE: usize = 32;
const MAX_REACTION_CHUNKS: usize = 16;

fn reaction_chunks(params: Value) -> Result<Vec<(Value, HashSet<String>)>, &'static str> {
    let own = params["own"].as_array().ok_or("addon-invalid-response")?;
    if own.len() > REACTION_CHUNK_SIZE * MAX_REACTION_CHUNKS {
        return Err("addon-reaction-over-budget");
    }
    own.chunks(REACTION_CHUNK_SIZE)
        .map(|objects| {
            let dependencies: HashSet<_> = objects
                .iter()
                .filter_map(|object| object["_reactionRefs"].as_array())
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let deleted_dependencies: HashSet<_> = objects
                .iter()
                .filter_map(|object| object["_reactionDeletedRefs"].as_array())
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let changed: Vec<_> = params["changed"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|object| {
                    object["id"]
                        .as_str()
                        .is_some_and(|id| dependencies.contains(id))
                })
                .collect();
            let deleted: Vec<_> = params["deleted"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|id| {
                    id.as_str()
                        .is_some_and(|id| deleted_dependencies.contains(id))
                })
                .collect();
            let own: Vec<_> = objects
                .iter()
                .cloned()
                .map(|mut object| {
                    object.as_object_mut().unwrap().remove("_reactionRefs");
                    object
                        .as_object_mut()
                        .unwrap()
                        .remove("_reactionDeletedRefs");
                    object
                })
                .collect();
            let chunk =
                json!({"page":params["page"], "own":own, "changed":changed, "deleted":deleted});
            let scope = objects
                .iter()
                .filter_map(|object| object["id"].as_str().map(str::to_owned))
                .collect();
            (chunk, scope)
        })
        .map(Ok)
        .collect()
}

fn run_reaction_chunks(
    root: &std::path::Path,
    snapshot: &storage::NotebookSnapshot,
    addon: &str,
    params: Value,
    mut invoke: impl FnMut(Value) -> Result<Value, &'static str>,
) -> Result<Option<storage::NotebookSnapshot>, &'static str> {
    let mut candidate = snapshot.clone();
    let mut updated = false;
    for (params, scope) in reaction_chunks(params)? {
        let value = invoke(params)?;
        if addon == super::catalog::CONNECTORS.id {
            batch::check_path_reaction(snapshot, addon, &value)
                .map_err(|_| "addon-invalid-response")?;
        }
        let operations = value
            .pointer("/batch/operations")
            .or_else(|| value.pointer("/batch/ops"))
            .or_else(|| value.get("ops"));
        if operations
            .and_then(Value::as_array)
            .is_some_and(|ops| ops.len() > 32)
        {
            return Err("addon-reaction-over-budget");
        }
        // Validate each reply against the captured ownership before combining it.
        batch::apply_owned(root, snapshot.clone(), addon, Some(&scope), value.clone())
            .map_err(|_| "addon-invalid-response")?;
        if let Some(next) = batch::apply_owned(root, candidate.clone(), addon, Some(&scope), value)
            .map_err(|_| "addon-invalid-response")?
        {
            candidate = next;
            updated = true;
        }
    }
    Ok(updated.then_some(candidate))
}

#[derive(Serialize, Default)]
pub(crate) struct ReactionResult {
    token: Option<String>,
    retry: bool,
    /// The addon whose reaction failed; the user commit is preserved.
    failed: Option<selection::Name>,
    owner: Option<selection::Name>,
    count: usize,
}

struct PendingReaction {
    token: String,
    root: String,
    page: String,
    candidate: storage::NotebookSnapshot,
    authorization: Vec<(&'static super::Addon, selection::Authorization)>,
    deadline: std::time::Instant,
    retry: bool,
    count: usize,
}

#[derive(Default)]
pub(crate) struct Reactions(std::sync::Mutex<Option<PendingReaction>>);

#[tauri::command]
pub(crate) async fn addon_react(
    app: tauri::AppHandle,
    root: String,
    page_id: String,
    revision: u64,
    base_revision: u64,
    retry: bool,
) -> Result<ReactionResult, ActionError> {
    use super::manifest::Permission;
    use std::time::{Duration, Instant};
    let captured = notebook::on_page(
        &app.state(),
        &app.state(),
        root.clone(),
        page_id.clone(),
        move |root, page, history| {
            let current = storage::open_page(root, page)?;
            let Some((before, committed)) = history.amendment_base(
                if retry {
                    current.page.revision
                } else {
                    revision
                },
                base_revision,
            ) else {
                return Ok(None);
            };
            if (!retry && current.page.revision != revision)
                || current.page.calculation.is_some()
                || !current
                    .page
                    .objects
                    .iter()
                    .any(|object| object.fields().addon_data.is_some())
            {
                return Ok(None);
            }
            if retry && committed.elapsed() < Duration::from_secs(1) {
                return Ok(None);
            }
            Ok(Some((before.clone(), current, committed)))
        },
    )
    .await?;
    let Some((before, mut current, committed)) = captured else {
        return Ok(ReactionResult::default());
    };
    let deadline = if retry {
        Instant::now() + Duration::from_secs(30)
    } else {
        committed + Duration::from_millis(750)
    };
    let mut claims = Vec::new();
    let mut affected_count = 0;
    let mut chunk_count = 0;
    let mut before = before;
    let mut visited = HashSet::new();
    for _ in 0..2 {
        let after = current.clone();
        let previous_claims = claims.len();
        for &addon in super::catalog::CATALOG {
            if visited.contains(addon.id)
                || !addon.permissions().contains(&Permission::ReactToChanges)
            {
                continue;
            }
            let Some((params, scope)) = reactions(&before, &after, addon.id) else {
                continue;
            };
            let count = scope.len();
            affected_count += count;
            chunk_count += count.div_ceil(REACTION_CHUNK_SIZE);
            if chunk_count > MAX_REACTION_CHUNKS {
                #[cfg(debug_assertions)]
                eprintln!(
                    "addon-reaction-failed addon={} count={} reason=addon-reaction-over-budget",
                    addon.id, count
                );
                return Ok(ReactionResult {
                    token: None,
                    retry: false,
                    failed: Some(selection::Name::of(addon)),
                    owner: None,
                    count,
                });
            }
            let handle = app.clone();
            let requested_root = root.clone();
            let context = format!("{root}:{page_id}");
            let snapshot = current.clone();
            let response = tauri::async_runtime::spawn_blocking(move || {
                let root =
                    workspace::ensure_allowed(&handle.state::<AllowedRoots>(), &requested_root)
                        .ok()?;
                let importers = handle.state::<Importers>();
                let (job, _) = importers.get(addon.id).ok()?;
                let authorization = selection::authorization(&handle, job, addon).ok()?;
                let candidate = run_reaction_chunks(&root, &snapshot, addon.id, params, |params| {
                    handle
                        .state::<session::Runtime>()
                        .run_scoped(
                            &handle,
                            addon,
                            "",
                            {
                                let invocation = Invocation::Reaction(params);
                                let reads = scoped_reads::Reads::capture(&snapshot, &invocation);
                                (invocation, reads)
                            },
                            Some(&context),
                            Some(snapshot.page.revision),
                            || {
                                Instant::now() >= deadline
                                    || selection::revoked(job, &authorization)
                            },
                        )
                        .map_err(|failure| failure.code())?
                        .map_err(|_| "addon-invalid-response")
                });
                if selection::revoked(job, &authorization) {
                    return Some((Err("addon-cancelled"), authorization));
                }
                Some((candidate, authorization))
            })
            .await
            .map_err(|_| error("addon-unavailable"))?;
            if Instant::now() >= deadline
                || response.as_ref().is_some_and(|(result, _)| result.is_err())
            {
                #[cfg(debug_assertions)]
                eprintln!(
                    "addon-reaction-failed addon={} count={} reason={}",
                    addon.id,
                    count,
                    response
                        .as_ref()
                        .and_then(|(result, _)| result.as_ref().err().copied())
                        .unwrap_or("addon-timeout")
                );
                return Ok(ReactionResult {
                    token: None,
                    retry: !retry,
                    failed: Some(selection::Name::of(addon)),
                    owner: None,
                    count,
                });
            }
            visited.insert(addon.id);
            let Some((Ok(Some(candidate)), authorization)) = response else {
                continue;
            };
            current = candidate;
            claims.push((addon, authorization));
        }
        if claims.len() == previous_claims {
            break;
        }
        before = after;
    }
    if claims.is_empty() {
        return Ok(ReactionResult::default());
    }
    let token = crate::helper::random_hex(16).map_err(error)?;
    let owner = claims.first().map(|(addon, _)| selection::Name::of(addon));
    *app.state::<Reactions>().0.lock().unwrap() = Some(PendingReaction {
        token: token.clone(),
        root,
        page: page_id,
        candidate: current,
        authorization: claims,
        deadline,
        retry,
        count: affected_count,
    });
    Ok(ReactionResult {
        token: Some(token),
        retry: false,
        failed: None,
        owner,
        count: affected_count,
    })
}

fn reaction_gate(
    expected: u64,
    actual: u64,
    expired: bool,
    retry: bool,
) -> Option<(&'static str, bool)> {
    if expected != actual {
        Some(("addon-stale-page", !retry))
    } else if expired {
        Some(("addon-timeout", !retry))
    } else {
        None
    }
}

#[tauri::command]
pub(crate) async fn addon_apply_reaction(
    app: tauri::AppHandle,
    token: String,
) -> Result<
    (
        Option<storage::HistoryResult>,
        bool,
        Option<selection::Name>,
        usize,
    ),
    ActionError,
> {
    let state = app.state::<Reactions>();
    let pending = {
        let mut slot = state.0.lock().unwrap();
        if slot.as_ref().is_none_or(|pending| pending.token != token) {
            return Ok((None, false, None, 0));
        }
        slot.take().unwrap()
    };
    let handle = app.clone();
    notebook::on_page(
        &app.state(),
        &app.state(),
        pending.root,
        pending.page,
        move |root, _, history| {
            let importers = handle.state::<Importers>();
            let mut guards = Vec::new();
            for (addon, authorization) in &pending.authorization {
                let (job, _) = importers.get(addon.id).map_err(|e| invalid(&e))?;
                let Some(state) = job.lock_idle() else {
                    return Ok((None, false, Some(selection::Name::of(addon)), pending.count));
                };
                if !selection::authorized(&handle, addon, &state)
                    .is_ok_and(|actual| actual == *authorization)
                {
                    return Ok((None, false, Some(selection::Name::of(addon)), pending.count));
                }
                guards.push(state);
            }
            let current = storage::open_page(root, &pending.candidate.page.id)?;
            if let Some((_reason, retry)) = reaction_gate(
                pending.candidate.page.revision,
                current.page.revision,
                std::time::Instant::now() >= pending.deadline,
                pending.retry,
            ) {
                #[cfg(debug_assertions)]
                eprintln!(
                    "addon-reaction-failed addon={} count={} reason={}",
                    pending
                        .authorization
                        .first()
                        .map(|(addon, _)| addon.id)
                        .unwrap_or("unknown"),
                    pending.count,
                    _reason
                );
                return Ok((
                    None,
                    retry,
                    pending
                        .authorization
                        .first()
                        .map(|(addon, _)| selection::Name::of(addon)),
                    pending.count,
                ));
            }
            storage::amend_notebook(root, history, pending.candidate)
                .map(|result| (Some(result), false, None, 0))
        },
    )
    .await
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> storage::NotebookSnapshot {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/notebooks/minimal");
        let mut snapshot = storage::open_notebook(&root).unwrap();
        let shape = |id: &str, x: f64| -> PageObject {
            serde_json::from_value(json!({
                "id":id,"type":"shape","x":x,"y":20.0,"rotation":0.0,"scale":1.0,
                "zIndex":1,"readingOrder":0,"groupId":null,"createdAt":"2026-11-23","modifiedAt":"2026-11-23",
                "geometry":{"kind":"rectangle","widthPt":20.0,"heightPt":10.0,"cornerRadiusPt":0.0},
                "style":{"strokeColor":"#000000","strokeWidthPt":1.0,"fillColor":null,"opacity":1.0}
            })).unwrap()
        };
        let mut connector = shape("connector", 70.0);
        connector.fields_mut().addon_data = Some(
            serde_json::from_value(json!({
                "connectors":{"refs":["left","right"],"data":{"mode":"straight"}},
                "another":{"refs":[],"data":{"private":true}}
            }))
            .unwrap(),
        );
        snapshot.page.objects = vec![
            shape("left", 20.0),
            shape("right", 100.0),
            shape("unrelated", 130.0),
            connector,
        ];
        snapshot
    }

    #[test]
    fn reaction_fanout_preserves_snapshot_and_bounds_scope() {
        let before = fixture();
        let mut after = before.clone();
        let connector = after.page.objects.last().unwrap().clone();
        for index in 0..64 {
            let mut object = connector.clone();
            object.fields_mut().id = format!("connector-{index}");
            after.page.objects.push(object);
        }
        after.page.objects[0].fields_mut().x += 10.0;
        let (params, scope) = reactions(&before, &after, "connectors").unwrap();
        let chunks = reaction_chunks(params.clone()).unwrap();
        assert_eq!(chunks.len(), 3);
        let mut combined = HashSet::new();
        for (chunk, ids) in chunks {
            assert_eq!(chunk["page"], params["page"]);
            assert_eq!(chunk["changed"][0]["id"], "left");
            assert!(chunk["own"][0].get("_reactionRefs").is_none());
            assert!(ids.len() <= 32);
            assert!(combined.is_disjoint(&ids));
            combined.extend(ids);
        }
        assert_eq!(combined, scope);
        let mut too_many = params;
        too_many["own"] = json!(vec![json!({"id":"x"}); 513]);
        assert_eq!(
            reaction_chunks(too_many).unwrap_err(),
            "addon-reaction-over-budget"
        );
    }

    #[test]
    fn reaction_chunks_disclose_only_disjoint_dependencies() {
        let mut before = fixture();
        let connector = before.page.objects.pop().unwrap();
        for index in 0..33 {
            let mut object = connector.clone();
            object.fields_mut().id = format!("connector-{index}");
            let refs = if index < 32 {
                vec!["left", "unrelated"]
            } else {
                vec!["right", "removed"]
            };
            object.fields_mut().addon_data = Some(
                serde_json::from_value(json!({
                    "connectors":{"refs":refs,"data":{}}
                }))
                .unwrap(),
            );
            before.page.objects.push(object);
        }
        let mut removed = before.page.objects[0].clone();
        removed.fields_mut().id = "removed".into();
        before.page.objects.push(removed);
        let mut after = before.clone();
        after
            .page
            .objects
            .retain(|object| !["unrelated", "removed"].contains(&object.fields().id.as_str()));
        addon_data::remove_refs(
            &mut after.page.objects,
            &HashSet::from(["unrelated", "removed"]),
        );
        after.page.objects[0].fields_mut().x += 10.0;
        after.page.objects[1].fields_mut().x += 20.0;
        let (params, _) = reactions(&before, &after, "connectors").unwrap();
        let chunks = reaction_chunks(params).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].0["changed"][0]["id"], "left");
        assert_eq!(chunks[1].0["changed"][0]["id"], "right");
        assert_eq!(chunks[0].0["changed"].as_array().unwrap().len(), 1);
        assert_eq!(chunks[1].0["changed"].as_array().unwrap().len(), 1);
        assert_eq!(chunks[0].0["deleted"], json!(["unrelated"]));
        assert_eq!(chunks[1].0["deleted"], json!(["removed"]));
        let unknown = reaction_chunks(json!({
            "page": {}, "own": [{"id":"unknown"}],
            "changed":[{"id":"secret"}], "deleted":["secret"], "extra":"private"
        }))
        .unwrap();
        assert_eq!(unknown[0].0["changed"], json!([]));
        assert_eq!(unknown[0].0["deleted"], json!([]));
        assert!(unknown[0].0.get("extra").is_none());
    }

    #[test]
    fn reaction_fanout_amends_once_and_failed_chunk_publishes_nothing() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("notebook");
        let mut initial = fixture();
        let connector = initial.page.objects.last().unwrap().clone();
        for index in 0..40 {
            let mut object = connector.clone();
            object.fields_mut().id = format!("connector-{index}");
            initial.page.objects.push(object);
        }
        for (index, object) in initial.page.objects.iter_mut().enumerate() {
            object.fields_mut().reading_order = index as u32;
        }
        initial.blocks.retain(|file| file.path == "style.typ");
        initial.assets.clear();
        initial.page.ink_layers.clear();
        initial.ink_layers.clear();
        initial.page.reading_order = initial
            .page
            .objects
            .iter()
            .map(|object| object.fields().id.clone())
            .collect();
        initial.page.schema_version = 5;
        initial.manifest.schema_version = 5;
        storage::create_notebook(&root, &initial).unwrap();
        let mut history = storage::NotebookHistory::default();
        let before = storage::observe_page(&root, &initial.page.id, &mut history).unwrap();
        let mut after = before.clone();
        after.page.objects[0].fields_mut().x += 10.0;
        let after = storage::commit_notebook(&root, &mut history, after)
            .unwrap()
            .snapshot;
        let (params, _) = reactions(&before, &after, "connectors").unwrap();
        let reply = |params: Value| -> Result<Value, &'static str> {
            let operations: Vec<_> = params["own"]
                .as_array()
                .unwrap()
                .iter()
                .map(|object| json!({"op":"updateObject","id":object["id"],"x":80.0}))
                .collect();
            Ok(json!({"batch":{"operations":operations}}))
        };
        let mut calls = 0;
        let failed = run_reaction_chunks(&root, &after, "connectors", params.clone(), |params| {
            calls += 1;
            if calls == 2 {
                return Err("addon-timeout");
            }
            reply(params)
        });
        assert_eq!(calls, 2);
        assert!(failed.is_err());
        assert_eq!(
            storage::open_page(&root, &after.page.id)
                .unwrap()
                .page
                .objects,
            after.page.objects
        );
        let candidate = run_reaction_chunks(&root, &after, "connectors", params, reply)
            .unwrap()
            .unwrap();
        assert_eq!(candidate.page.revision, after.page.revision);
        assert!(
            candidate.page.objects[3..]
                .iter()
                .all(|object| object.fields().x == 80.0)
        );
        storage::amend_notebook(&root, &mut history, candidate).unwrap();
        let undone = storage::undo_notebook(&root, &mut history).unwrap();
        assert_eq!(undone.snapshot.page.objects, before.page.objects);
        assert!(!undone.can_undo);
    }

    #[test]
    fn stale_reaction_retries_new_revision_once_then_reports_failure() {
        // Exercise the production fan-out and history functions against persisted revisions.
        for stale_twice in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("notebook");
            let mut initial = fixture();
            let connector = initial.page.objects.last().unwrap().clone();
            for index in 0..32 {
                let mut object = connector.clone();
                object.fields_mut().id = format!("connector-{index}");
                initial.page.objects.push(object);
            }
            for (index, object) in initial.page.objects.iter_mut().enumerate() {
                object.fields_mut().reading_order = index as u32;
            }
            initial.blocks.retain(|file| file.path == "style.typ");
            initial.assets.clear();
            initial.page.ink_layers.clear();
            initial.ink_layers.clear();
            initial.page.reading_order = initial
                .page
                .objects
                .iter()
                .map(|object| object.fields().id.clone())
                .collect();
            initial.page.schema_version = 5;
            initial.manifest.schema_version = 5;
            storage::create_notebook(&root, &initial).unwrap();
            let mut history = storage::NotebookHistory::default();
            let before = storage::observe_page(&root, &initial.page.id, &mut history).unwrap();
            let base_revision = before.page.revision;
            let mut edit = before.clone();
            edit.page.objects[0].fields_mut().x += 10.0;
            let mut captured = storage::commit_notebook(&root, &mut history, edit)
                .unwrap()
                .snapshot;
            let mut calls = 0;
            for retry in [false, true] {
                let (base, _) = history
                    .amendment_base(captured.page.revision, base_revision)
                    .unwrap();
                let (params, _) = reactions(base, &captured, "connectors").unwrap();
                let expected_x = captured.page.objects[0].fields().x;
                let candidate = run_reaction_chunks(&root, &captured, "connectors", params, |params| {
                    calls += 1;
                    assert_eq!(params["changed"][0]["x"], expected_x);
                    let operations: Vec<_> = params["own"].as_array().unwrap().iter()
                        .map(|object| json!({"op":"updateObject","id":object["id"],"x":expected_x + 50.0})).collect();
                    // An edit lands while the first chunk is in flight.
                    if (!retry || stale_twice) && calls % 2 == 1 {
                        let mut edit = storage::open_page(&root, &captured.page.id).unwrap();
                        edit.page.objects[0].fields_mut().x += 5.0;
                        storage::commit_notebook(&root, &mut history, edit).unwrap();
                    }
                    Ok(json!({"batch":{"operations":operations}}))
                }).unwrap().unwrap();
                let current = storage::open_page(&root, &captured.page.id).unwrap();
                let gate =
                    reaction_gate(candidate.page.revision, current.page.revision, false, retry);
                if !retry || stale_twice {
                    assert_eq!(gate, Some(("addon-stale-page", !retry)));
                    assert!(
                        current.page.objects[3..]
                            .iter()
                            .all(|object| object.fields().x == 70.0)
                    );
                    captured = current;
                } else {
                    assert_eq!(gate, None);
                    let amended = storage::amend_notebook(&root, &mut history, candidate).unwrap();
                    assert!(
                        amended.snapshot.page.objects[3..]
                            .iter()
                            .all(|object| object.fields().x == expected_x + 50.0)
                    );
                    let undone = storage::undo_notebook(&root, &mut history).unwrap();
                    assert!(
                        undone.snapshot.page.objects[3..]
                            .iter()
                            .all(|object| object.fields().x == 70.0)
                    );
                }
            }
            assert_eq!(calls, 4);
        }
    }

    #[test]
    fn capture_resolves_snap_positions_without_disclosing_whole_objects() {
        let snapshot = fixture();
        for points in [
            vec![Point {
                x: 0.0,
                y: 0.0,
                attach: None,
            }],
            vec![
                Point {
                    x: 0.0,
                    y: 0.0,
                    attach: Some(Attachment {
                        object_id: "left".into(),
                        port: 1,
                    }),
                },
                Point {
                    x: 1.0,
                    y: 2.0,
                    attach: None,
                },
            ],
        ] {
            let count = points.len();
            let (_, Invocation::Tool { points, params, .. }, _) = capture(
                &snapshot,
                "connectors",
                Action::Tool {
                    id: "fixture".into(),
                    points,
                    style: None,
                    pen: Some(Pen {
                        stroke_color: "#123abc".into(),
                        stroke_width_pt: 1.5,
                    }),
                },
            )
            .unwrap() else {
                panic!("tool scope");
            };
            assert_eq!(points, count);
            assert_eq!(
                params["pen"],
                json!({"strokeColor": "#123abc", "strokeWidthPt": 1.5})
            );
            if count == 2 {
                assert_eq!(params["points"][0]["x"], 40.0);
                assert_eq!(params["points"][0]["y"], 25.0);
                let objects = params["objects"].as_array().unwrap();
                assert_eq!(objects.len(), 1);
                assert_eq!(objects[0]["id"], "left");
                assert!(objects[0].get("style").is_none());
            }
        }
    }

    #[test]
    fn tool_capture_validates_the_core_shape_style_instead_of_a_pen() {
        let snapshot = fixture();
        for (width, valid) in [(2.5, true), (-1.0, false)] {
            let style = json!({"strokeColor":"#123abc", "strokeWidthPt":width, "fillColor":null,
                "opacity":0.8, "pattern":"dotted", "patternSpacing":1.5});
            let action = serde_json::from_value(json!({"kind":"tool", "id":"fixture",
                "points":[{"x":1.0, "y":2.0, "attach":null}], "style":style}))
            .unwrap();
            match capture(&snapshot, "connectors", action) {
                Ok((_, Invocation::Tool { params, .. }, _)) if valid => {
                    assert_eq!(params["style"], style);
                    assert!(params.get("pen").is_none());
                }
                Err(_) if !valid => {}
                _ => panic!("unexpected tool style validation"),
            }
        }
    }

    #[test]
    fn tool_capture_rejects_an_invalid_pen() {
        let snapshot = fixture();
        for pen in [("red", 1.0), ("#123abc", f64::NAN), ("#123abc", 0.0)] {
            let action = Action::Tool {
                id: "fixture".into(),
                points: vec![Point {
                    x: 1.0,
                    y: 2.0,
                    attach: None,
                }],
                style: None,
                pen: Some(Pen {
                    stroke_color: pen.0.into(),
                    stroke_width_pt: pen.1,
                }),
            };
            assert!(capture(&snapshot, "connectors", action).is_err());
        }
    }

    #[test]
    fn ink_reference_snapshots_use_transformed_stroke_bounds() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/notebooks/minimal");
        let mut snapshot = storage::open_notebook(&root).unwrap();
        let object = snapshot
            .page
            .objects
            .iter()
            .find(|object| matches!(object, PageObject::InkGroup { .. }))
            .unwrap();
        let (left, _, right, _) = {
            let [left, top, right, bottom] = object_extent(object, &snapshot);
            (left, top, right, bottom)
        };
        assert!(right > left);
        snapshot.ink_layers[0].strokes[0].transform.translate_x += 10.0;
        assert_eq!(object_extent(object, &snapshot)[0], left + 10.0);
    }

    #[test]
    fn addon_metadata_has_no_effect_on_render_or_export_input() {
        let snapshot = fixture();
        let mut plain = snapshot.clone();
        for object in &mut plain.page.objects {
            object.fields_mut().addon_data = None;
            object.fields_mut().ports = None;
        }
        assert_eq!(
            crate::export::export_page_from_bundle(&snapshot, false).unwrap(),
            crate::export::export_page_from_bundle(&plain, false).unwrap()
        );
    }

    #[test]
    fn stretch_notifies_connector_reactions_with_both_group_ports() {
        use goodtype_core::stretch::{self, Limits, Part, Role, StretchRequest, StretchRule};
        let mut before = fixture();
        let child = &mut before.page.objects[0];
        let mut fields = child.fields().clone();
        child.fields_mut().group_id = Some("symbol".into());
        fields.id = "symbol".into();
        fields.ports = Some(vec![
            ShapePoint { x: 0.0, y: 5.0 },
            ShapePoint { x: 20.0, y: 5.0 },
        ]);
        before.page.objects.push(PageObject::Group {
            fields,
            child_ids: vec!["left".into()],
            stretch: Some(StretchRule {
                axis: [0, 1],
                parts: vec![Part {
                    child_id: "left".into(),
                    role: Role::Span,
                }],
                limits: Limits::Span {
                    minimum_length: 10.0,
                },
            }),
        });
        let start = &mut before.page.objects[3];
        start.fields_mut().addon_data = Some(
            serde_json::from_value(json!({
                "connectors": {"refs":["symbol"],"data":{"attach":{"objectId":"symbol","port":0}}}
            }))
            .unwrap(),
        );
        let mut end = start.clone();
        end.fields_mut().id = "end-connector".into();
        end.fields_mut().addon_data = Some(
            serde_json::from_value(json!({
                "connectors": {"refs":["symbol"],"data":{"attach":{"objectId":"symbol","port":1}}}
            }))
            .unwrap(),
        );
        before.page.objects.push(end);
        for (port, x, expected) in [(0, 10.0, [10.0, 40.0]), (1, 60.0, [20.0, 60.0])] {
            let mut after = before.clone();
            assert!(
                stretch::apply(
                    &mut after.page.objects,
                    &StretchRequest {
                        page_id: before.page.id.clone(),
                        revision: before.page.revision,
                        group_id: "symbol".into(),
                        port,
                        endpoint: ShapePoint { x, y: 25.0 },
                        modified_at: "2026-11-25".into(),
                    }
                )
                .unwrap()
            );
            let (payload, scope) = reactions(&before, &after, "connectors").unwrap();
            assert_eq!(
                scope,
                HashSet::from(["connector".to_owned(), "end-connector".to_owned()])
            );
            assert_eq!(payload["deleted"], json!([]));
            assert_eq!(payload["changed"].as_array().unwrap().len(), 1);
            assert_eq!(payload["changed"][0]["id"], "symbol");
            assert_eq!(
                payload["changed"][0]["ports"],
                json!([
                    {"x":expected[0],"y":25.0}, {"x":expected[1],"y":25.0},
                ])
            );
            for index in 0..2 {
                assert_eq!(
                    payload["own"][index]["addonData"]["connectors"]["data"]["attach"],
                    json!({"objectId":"symbol","port":index})
                );
            }
            assert_eq!(before.page.objects[3], after.page.objects[3]);
            assert_eq!(before.page.objects[5], after.page.objects[5]);
        }
    }

    #[test]
    fn reactions_scope_changed_deleted_and_own_objects() {
        let before = fixture();
        let mut after = before.clone();
        after.page.objects[0].fields_mut().x += 10.0;
        after.page.objects.remove(1);
        after.page.objects[1].fields_mut().x += 5.0;
        addon_data::remove_refs(&mut after.page.objects, &HashSet::from(["right"]));
        let (payload, scope) = reactions(&before, &after, "connectors").unwrap();
        assert_eq!(scope, HashSet::from(["connector".to_string()]));
        assert_eq!(payload["deleted"], json!(["right"]));
        let changed = payload["changed"].as_array().unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0]["id"], "left");
        assert_eq!(changed[0]["ports"][1]["x"], 50.0);
        let own = &payload["own"][0];
        assert_eq!(own["addonData"]["connectors"]["refs"], json!(["left"]));
        assert!(own["addonData"].get("another").is_none());
        assert!(reactions(&before, &after, "another").is_none());
    }
}
