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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    root: String,
    page_id: String,
    revision: u64,
    addon_id: String,
    action: Action,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Action {
    Tool {
        id: String,
        points: Vec<Point>,
        #[serde(default)]
        pen: Option<Pen>,
        #[serde(default)]
        style: Option<goodtype_core::ShapeStyle>,
    },
    Command {
        id: String,
        selected: Vec<String>,
    },
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
struct Attachment {
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

fn capture(
    bundle: &storage::NotebookSnapshot,
    addon: &str,
    action: Action,
) -> Result<(String, Invocation, Option<HashSet<String>>), String> {
    let page = &bundle.page;
    let objects = &page.objects;
    let find = |id: &str| objects.iter().find(|obj| obj.fields().id == id);
    match action {
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
                    let object = find(&attachment.object_id).ok_or("addon-object-permission")?;
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
            message: format!("{error:?}"),
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
) -> Result<Option<storage::HistoryResult>, ActionError> {
    let handle = app.clone();
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
        let (command, invocation, scope) =
            capture(&snapshot, addon.id, request.action).map_err(error)?;
        let context = format!("{}:{}", root.display(), request.page_id);
        let reply = app
            .state::<session::Runtime>()
            .run(&app, addon, &command, invocation, Some(&context), || {
                selection::revoked(job, &authorization)
            })
            .map_err(|failure| error(failure.code()))?
            .map_err(|failure| ActionError {
                code: failure.code,
                message: failure.message,
            })?;
        let candidate =
            batch::apply_owned(&root, snapshot, addon.id, scope.as_ref(), reply).map_err(error)?;
        Ok::<_, ActionError>((
            request.root,
            request.page_id,
            request.addon_id,
            candidate,
            base,
            authorization,
        ))
    })
    .await
    .map_err(|_| error("addon-unavailable"))??;
    let (root, page, addon_id, candidate, base, authorization) = prepared;
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let handle = app.clone();
    notebook::on_page(
        &app.state(),
        &app.state(),
        root,
        page,
        move |root, _, history| {
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
            storage::commit_notebook_with_precondition(
                root,
                history,
                candidate,
                Some(storage::ProposalPrecondition {
                    base,
                    require_retained_history: false,
                }),
            )
            .map(Some)
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
            own.push(own_value(object, addon));
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

#[derive(Serialize, Default)]
pub(crate) struct ReactionResult {
    token: Option<String>,
    retry: bool,
}

struct PendingReaction {
    token: String,
    root: String,
    page: String,
    candidate: storage::NotebookSnapshot,
    authorization: Vec<(&'static super::Addon, selection::Authorization)>,
    deadline: std::time::Instant,
    retry: bool,
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
            let Some((before, committed)) = history.amendment_base(revision, base_revision) else {
                return Ok(None);
            };
            if current.page.revision != revision
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
                let value = handle
                    .state::<session::Runtime>()
                    .run(
                        &handle,
                        addon,
                        "",
                        Invocation::Reaction(params),
                        Some(&context),
                        || Instant::now() >= deadline || selection::revoked(job, &authorization),
                    )
                    .ok()?
                    .ok()?;
                let candidate =
                    batch::apply_owned(&root, snapshot, addon.id, Some(&scope), value).ok()?;
                Some((candidate, authorization))
            })
            .await
            .map_err(|_| error("addon-unavailable"))?;
            if Instant::now() >= deadline {
                return Ok(ReactionResult {
                    token: None,
                    retry: !retry,
                });
            }
            visited.insert(addon.id);
            let Some((Some(candidate), authorization)) = response else {
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
    *app.state::<Reactions>().0.lock().unwrap() = Some(PendingReaction {
        token: token.clone(),
        root,
        page: page_id,
        candidate: current,
        authorization: claims,
        deadline,
        retry,
    });
    Ok(ReactionResult {
        token: Some(token),
        retry: false,
    })
}

#[tauri::command]
pub(crate) async fn addon_apply_reaction(
    app: tauri::AppHandle,
    token: String,
) -> Result<(Option<storage::HistoryResult>, bool), ActionError> {
    let state = app.state::<Reactions>();
    let pending = {
        let mut slot = state.0.lock().unwrap();
        if slot.as_ref().is_none_or(|pending| pending.token != token) {
            return Ok((None, false));
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
            if std::time::Instant::now() >= pending.deadline {
                return Ok((None, !pending.retry));
            }
            let importers = handle.state::<Importers>();
            let mut guards = Vec::new();
            for (addon, authorization) in &pending.authorization {
                let (job, _) = importers.get(addon.id).map_err(|e| invalid(&e))?;
                let Some(state) = job.lock_idle() else {
                    return Ok((None, false));
                };
                if !selection::authorized(&handle, addon, &state)
                    .is_ok_and(|actual| actual == *authorization)
                {
                    return Ok((None, false));
                }
                guards.push(state);
            }
            storage::amend_notebook(root, history, pending.candidate)
                .map(|result| (Some(result), false))
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
