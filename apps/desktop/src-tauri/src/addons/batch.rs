use super::{selection, *};
use crate::helper::proposals::ProposalItem;
use goodtype_core::addon_data::{AddonData, AddonEntry};
use goodtype_core::{
    ObjectFields, PageBackground, PageGeometry, PageObject, ShapeGeometry, ShapePoint, ShapeStyle,
    storage, template,
};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    batch: Batch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    operations: Vec<Operation>,
}

#[derive(Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Operation {
    #[serde(rename = "addTypstBlock")]
    TypstBlock {
        source: String,
        x_pt: f64,
        y_pt: f64,
        layout_width_pt: f64,
        addon_data: Option<AddonData>,
    },
    #[serde(rename = "addShape")]
    Shape {
        x: f64,
        y: f64,
        geometry: ShapeGeometry,
        style: ShapeStyle,
        addon_data: Option<AddonData>,
        ports: Option<Vec<ShapePoint>>,
    },
    #[serde(rename = "addGroup")]
    Group {
        children: Vec<Operation>,
        addon_data: Option<AddonData>,
        ports: Option<Vec<ShapePoint>>,
    },
    UpdateObject {
        id: String,
        x: Option<f64>,
        y: Option<f64>,
        rotation: Option<f64>,
        scale: Option<f64>,
        geometry: Option<ShapeGeometry>,
        style: Option<ShapeStyle>,
        source: Option<String>,
        ports: Option<Vec<ShapePoint>>,
        #[serde(default, deserialize_with = "present_entry")]
        addon_data: Option<Option<AddonEntry>>,
    },
    DeleteObject {
        id: String,
    },
    #[serde(rename = "addPage")]
    Page {
        template: Option<template::PageTemplate>,
        ops: Vec<Operation>,
    },
}

#[derive(Clone, Serialize)]
pub(super) struct PreviewPage {
    background: PageBackground,
    items: Vec<ProposalItem>,
}

const INVALID: &str = "addon-invalid-response";

pub(super) fn prepare(
    root: &Path,
    geometry: &PageGeometry,
    reply: Value,
    generator: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<PreviewPage>, String> {
    let Response {
        batch: Batch { operations },
    } = serde_json::from_value(reply).map_err(|_| INVALID)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let check = || {
        if cancelled() {
            return Err("addon-cancelled");
        }
        if Instant::now() >= deadline {
            return Err("addon-timeout");
        }
        Ok(())
    };
    let mut budget = 32;
    if generator {
        if !(1..=16).contains(&operations.len()) {
            return Err(INVALID.into());
        }
        operations
            .into_iter()
            .map(|op| {
                let Operation::Page { template, ops } = op else {
                    return Err(INVALID.into());
                };
                let background = if let Some(mut template) = template {
                    template.id = crate::helper::random_hex(16)?;
                    template::validate(&template).map_err(|_| INVALID)?;
                    PageBackground::Template { template }
                } else {
                    white()
                };
                Ok(PreviewPage {
                    background,
                    items: legacy_items(objects(root, geometry, ops, &mut budget, &check)?)?,
                })
            })
            .collect()
    } else {
        if operations.is_empty() {
            return Err(INVALID.into());
        }
        Ok(vec![PreviewPage {
            background: white(),
            items: legacy_items(objects(root, geometry, operations, &mut budget, &check)?)?,
        }])
    }
}

fn present_entry<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<Option<AddonEntry>>, D::Error> {
    Option::deserialize(decoder).map(Some)
}

fn legacy_items(items: Vec<ProposalItem>) -> Result<Vec<ProposalItem>, String> {
    if items
        .iter()
        .any(|item| goodtype_core::addon_data::uses_addon_fields(item.object.fields()))
    {
        return Err(INVALID.into());
    }
    Ok(items)
}

pub(super) const FORBIDDEN: &str = "addon-object-permission";

/// Build a complete candidate before publishing any changes. Scope is derived from
/// the pre-invocation page, so a reply cannot grant itself access to another object.
pub(super) fn apply_owned(
    root: &Path,
    mut snapshot: storage::NotebookSnapshot,
    addon: &str,
    allowed: Option<&HashSet<String>>,
    reply: Value,
) -> Result<Option<storage::NotebookSnapshot>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OwnedBatch {
        ops: Vec<Operation>,
    }

    let wire = reply.get("batch").cloned().unwrap_or(reply);
    let OwnedBatch { ops } = if wire.get("operations").is_some() {
        let Batch { operations } = serde_json::from_value(wire).map_err(|_| INVALID)?;
        OwnedBatch { ops: operations }
    } else {
        serde_json::from_value(wire).map_err(|_| INVALID)?
    };
    if ops.is_empty() {
        return Ok(None);
    }
    let own: HashSet<_> = snapshot
        .page
        .objects
        .iter()
        .filter_map(|object| {
            let fields = object.fields();
            (fields.addon_data.as_ref()?.contains_key(addon)
                && allowed.is_none_or(|ids| ids.contains(&fields.id)))
            .then(|| fields.id.clone())
        })
        .collect();
    let mut budget = 32usize;
    let mut deleted = HashSet::new();
    let mut touched = HashSet::new();
    for op in ops {
        match op {
            Operation::UpdateObject {
                id,
                x,
                y,
                rotation,
                scale,
                geometry,
                style,
                source,
                ports,
                addon_data,
            } => {
                budget = budget.checked_sub(1).ok_or(INVALID)?;
                if !own.contains(&id) {
                    return Err(FORBIDDEN.into());
                }
                if !touched.insert(id.clone()) {
                    return Err(INVALID.into());
                }
                let object = snapshot
                    .page
                    .objects
                    .iter_mut()
                    .find(|object| object.fields().id == id)
                    .ok_or(INVALID)?;
                let fields = object.fields_mut();
                if let Some(x) = x {
                    fields.x = x;
                }
                if let Some(y) = y {
                    fields.y = y;
                }
                if let Some(rotation) = rotation {
                    fields.rotation = rotation;
                }
                if let Some(scale) = scale {
                    fields.scale = scale;
                }
                if let Some(ports) = ports {
                    fields.ports = Some(ports);
                }
                if let Some(entry) = addon_data {
                    if let Some(entry) = entry {
                        fields
                            .addon_data
                            .get_or_insert_with(AddonData::new)
                            .insert(addon.into(), entry);
                    } else {
                        fields.addon_data =
                            goodtype_core::addon_data::detach(fields.addon_data.take(), addon);
                    }
                }
                fields.modified_at = crate::helper::now().to_string();
                if geometry.is_some() || style.is_some() {
                    let PageObject::Shape {
                        geometry: old_geometry,
                        style: old_style,
                        ..
                    } = object
                    else {
                        return Err(INVALID.into());
                    };
                    if let Some(geometry) = geometry {
                        *old_geometry = geometry;
                    }
                    if let Some(style) = style {
                        *old_style = style;
                    }
                    validate_shape(old_geometry, old_style).map_err(|_| INVALID)?;
                }
                if let Some(source) = source {
                    let PageObject::Typst {
                        fields,
                        source_path,
                        layout_width_pt,
                        measured_width_pt,
                        measured_height_pt,
                    } = object
                    else {
                        return Err(INVALID.into());
                    };
                    let item = selection::compile(
                        root,
                        &snapshot.page.geometry,
                        fields.id.clone(),
                        selection::Operation::AddTypstBlock {
                            source,
                            x_pt: fields.x,
                            y_pt: fields.y,
                            layout_width_pt: *layout_width_pt,
                        },
                    )?;
                    *measured_width_pt = item.width_pt;
                    *measured_height_pt = item.height_pt;
                    let file = snapshot
                        .blocks
                        .iter_mut()
                        .find(|file| file.path == *source_path)
                        .ok_or(INVALID)?;
                    file.bytes = item.source.into_bytes();
                }
            }
            Operation::DeleteObject { id } => {
                budget = budget.checked_sub(1).ok_or(INVALID)?;
                if !own.contains(&id) {
                    return Err(FORBIDDEN.into());
                }
                if !touched.insert(id.clone()) {
                    return Err(INVALID.into());
                }
                deleted.insert(id);
            }
            op => {
                if allowed.is_some() {
                    return Err(FORBIDDEN.into());
                }
                let items = objects(
                    root,
                    &snapshot.page.geometry,
                    vec![op],
                    &mut budget,
                    &|| Ok(()),
                )?;
                if items.iter().any(|item| {
                    item.object
                        .fields()
                        .addon_data
                        .as_ref()
                        .is_some_and(|data| data.keys().any(|key| key != addon))
                }) {
                    return Err(FORBIDDEN.into());
                }
                PreviewPage {
                    background: white(),
                    items,
                }
                .insert(&mut snapshot)?;
            }
        }
    }
    // Empty parent groups and stroke membership are ordinary page invariants of a delete.
    loop {
        let emptied = snapshot
            .page
            .objects
            .iter()
            .filter_map(|object| {
                let PageObject::Group {
                    fields, child_ids, ..
                } = object
                else {
                    return None;
                };
                (!deleted.contains(&fields.id) && child_ids.iter().all(|id| deleted.contains(id)))
                    .then(|| fields.id.clone())
            })
            .collect::<Vec<_>>();
        if emptied.is_empty() {
            break;
        }
        deleted.extend(emptied);
    }
    for stroke in snapshot
        .ink_layers
        .iter_mut()
        .flat_map(|layer| &mut layer.strokes)
    {
        if stroke
            .group_id
            .as_ref()
            .is_some_and(|id| deleted.contains(id))
        {
            stroke.group_id = None;
        }
    }
    let removed_paths: HashSet<_> = snapshot
        .page
        .objects
        .iter()
        .filter(|object| deleted.contains(&object.fields().id))
        .filter_map(|object| object.searchable_source().map(str::to_owned))
        .collect();
    snapshot
        .page
        .objects
        .retain(|object| !deleted.contains(&object.fields().id));
    snapshot
        .page
        .reading_order
        .retain(|id| !deleted.contains(id));
    snapshot.blocks.retain(|file| {
        !removed_paths.contains(&file.path)
            || snapshot
                .page
                .objects
                .iter()
                .any(|object| object.searchable_source() == Some(file.path.as_str()))
    });
    for object in &mut snapshot.page.objects {
        let fields = object.fields_mut();
        if fields
            .group_id
            .as_ref()
            .is_some_and(|id| deleted.contains(id))
        {
            fields.group_id = None;
            snapshot.page.reading_order.push(fields.id.clone());
        }
        if let PageObject::Group { child_ids, .. } = object {
            child_ids.retain(|id| !deleted.contains(id));
        }
    }
    let deleted_refs = deleted.iter().map(String::as_str).collect();
    goodtype_core::addon_data::remove_refs(&mut snapshot.page.objects, &deleted_refs);
    let mut ordered: std::collections::HashMap<_, _> = snapshot
        .page
        .reading_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    for object in &mut snapshot.page.objects {
        let fields = object.fields_mut();
        if let Some(index) = ordered.remove(fields.id.as_str()) {
            fields.reading_order = index.try_into().map_err(|_| INVALID)?;
        }
    }
    storage::raise_schema_version(&mut snapshot);
    Ok(Some(snapshot))
}

fn white() -> PageBackground {
    PageBackground::Plain {
        color: "#ffffff".into(),
    }
}

fn objects(
    root: &Path,
    geometry: &PageGeometry,
    ops: Vec<Operation>,
    budget: &mut usize,
    check: &impl Fn() -> Result<(), &'static str>,
) -> Result<Vec<ProposalItem>, String> {
    let mut items = Vec::new();
    for op in ops {
        check()?;
        *budget = budget.checked_sub(1).ok_or(INVALID)?;
        if let Operation::Group {
            children,
            addon_data,
            ports,
        } = op
        {
            if children.is_empty() {
                return Err(INVALID.into());
            }
            *budget = budget.checked_sub(children.len()).ok_or(INVALID)?;
            let mut fields = fresh_fields(0.0, 0.0)?;
            fields.addon_data = addon_data;
            fields.ports = ports;
            let mut child_ids = Vec::new();
            for child in children {
                check()?;
                let mut item = leaf(root, geometry, child)?;
                let child = item.object.fields_mut();
                child.group_id = Some(fields.id.clone());
                child_ids.push(child.id.clone());
                items.push(item);
            }
            items.push(preview_item(PageObject::Group {
                fields,
                child_ids,
                stretch: None,
            }));
        } else {
            items.push(leaf(root, geometry, op)?);
        }
    }
    Ok(items)
}

fn leaf(root: &Path, page: &PageGeometry, op: Operation) -> Result<ProposalItem, String> {
    match op {
        Operation::TypstBlock {
            source,
            x_pt,
            y_pt,
            layout_width_pt,
            addon_data,
        } => {
            if x_pt < 0.0
                || y_pt < 0.0
                || layout_width_pt <= 0.0
                || x_pt + layout_width_pt > page.width_pt
                || y_pt >= page.height_pt
            {
                return Err(INVALID.into());
            }
            let op = selection::Operation::AddTypstBlock {
                source,
                x_pt,
                y_pt,
                layout_width_pt,
            };
            let mut item = selection::compile(
                root,
                page,
                format!("addon-{}", crate::helper::random_hex(16)?),
                op,
            )?;
            item.object.fields_mut().addon_data = addon_data;
            Ok(item)
        }
        Operation::Shape {
            x,
            y,
            geometry,
            style,
            addon_data,
            ports,
        } => {
            let area = (page.width_pt, page.height_pt, style.stroke_width_pt / 2.0);
            let mut fields = fresh_fields(x, y)?;
            fields.addon_data = addon_data;
            fields.ports = ports;
            let object = shape_object(fields, geometry, style).map_err(|_| INVALID)?;
            let PageObject::Shape { geometry, .. } = &object else {
                unreachable!("shape_object builds a shape");
            };
            if !shape_within(x, y, geometry, area) {
                return Err("addon-does-not-fit".into());
            }
            Ok(preview_item(object))
        }
        _ => Err(INVALID.into()),
    }
}

/// The one validation path for addon shapes, used by `addShape` batches and content packs.
pub(super) fn validate_shape(
    geometry: &ShapeGeometry,
    style: &ShapeStyle,
) -> Result<(), &'static str> {
    geometry.validate()?;
    style.validate_for(geometry)
}

/// The one construction path for addon shapes: an ordinary shape object after validation.
pub(super) fn shape_object(
    fields: ObjectFields,
    geometry: ShapeGeometry,
    style: ShapeStyle,
) -> Result<PageObject, &'static str> {
    validate_shape(&geometry, &style)?;
    Ok(PageObject::Shape {
        fields,
        geometry,
        style,
    })
}

/// Whether the drawn outline at `(x, y)` stays inside `width × height`, keeping `pad` from each
/// edge. Splines are measured by their curve extrema, so a handle may lie outside.
pub(super) fn shape_within(
    x: f64,
    y: f64,
    shape: &ShapeGeometry,
    (width, height, pad): (f64, f64, f64),
) -> bool {
    const EPSILON: f64 = 1e-9;
    shape_extent(shape).is_some_and(|[left, top, right, bottom]| {
        x + left >= pad - EPSILON
            && y + top >= pad - EPSILON
            && x + right <= width - pad + EPSILON
            && y + bottom <= height - pad + EPSILON
    })
}

/// `[left, top, right, bottom]` of the drawn outline relative to the object origin.
pub(super) fn shape_extent(shape: &ShapeGeometry) -> Option<[f64; 4]> {
    let extent = match shape {
        ShapeGeometry::Rectangle {
            width_pt,
            height_pt,
            ..
        }
        | ShapeGeometry::Ellipse {
            width_pt,
            height_pt,
        } => [0.0, 0.0, *width_pt, *height_pt],
        ShapeGeometry::Line { start, end } => [
            start.x.min(end.x),
            start.y.min(end.y),
            start.x.max(end.x),
            start.y.max(end.y),
        ],
        ShapeGeometry::Spline { nodes, closed } => {
            let mut extent = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            let add = |point: ShapePoint, offset: ShapePoint| ShapePoint {
                x: point.x + offset.x,
                y: point.y + offset.y,
            };
            let successors = nodes
                .iter()
                .skip(1)
                .chain(nodes.first().filter(|_| *closed));
            for (from, to) in nodes.iter().zip(successors) {
                let (p0, p3) = (from.point, to.point);
                let p1 = from.handle_out.map_or(p0, |handle| add(p0, handle));
                let p2 = to.handle_in.map_or(p3, |handle| add(p3, handle));
                let (left, right) = cubic_range([p0.x, p1.x, p2.x, p3.x]);
                let (top, bottom) = cubic_range([p0.y, p1.y, p2.y, p3.y]);
                extent = [
                    extent[0].min(left),
                    extent[1].min(top),
                    extent[2].max(right),
                    extent[3].max(bottom),
                ];
            }
            extent
        }
    };
    extent
        .iter()
        .all(|value| value.is_finite())
        .then_some(extent)
}

/// The range one axis of a cubic Bézier covers: its endpoints and the interior roots of its
/// derivative.
pub(super) fn cubic_range(points: [f64; 4]) -> (f64, f64) {
    let [start, _, _, end] = points;
    let (mut low, mut high) = (start.min(end), start.max(end));
    // Normalize before forming differences to avoid overflow for large finite handles.
    let norm = points.iter().copied().map(f64::abs).fold(1.0, f64::max);
    let [p, q, r, s] = points.map(|value| value / norm);
    let (first, middle, last) = (q - p, r - q, s - r);
    let (a, b, c) = (first - 2.0 * middle + last, 2.0 * (middle - first), first);
    let mut roots = [f64::NAN; 2];
    if a == 0.0 {
        roots[0] = -c / b;
    } else {
        let discriminant = b * b - 4.0 * a * c;
        if discriminant >= 0.0 {
            let ratio = -0.5 * (b + discriminant.sqrt().copysign(b));
            roots = [ratio / a, c / ratio];
        }
    }
    for t in roots.into_iter().filter(|t| 0.0 < *t && *t < 1.0) {
        let u = 1.0 - t;
        let value =
            (u * u * u * p + 3.0 * u * u * t * q + 3.0 * u * t * t * r + t * t * t * s) * norm;
        low = low.min(value);
        high = high.max(value);
    }
    (low, high)
}

fn fresh_fields(x: f64, y: f64) -> Result<ObjectFields, String> {
    let timestamp = crate::helper::now().to_string();
    Ok(ObjectFields {
        id: format!("addon-{}", crate::helper::random_hex(16)?),
        x,
        y,
        rotation: 0.0,
        scale: 1.0,
        z_index: 0,
        reading_order: 0,
        group_id: None,
        created_at: timestamp.clone(),
        modified_at: timestamp,
        addon_data: None,
        ports: None,
    })
}

fn preview_item(object: PageObject) -> ProposalItem {
    ProposalItem {
        placement: json!({"x": object.fields().x, "y": object.fields().y}),
        object,
        source: String::new(),
        hash: String::new(),
        svg: String::new(),
        before_source: None,
        before_svg: None,
        width_pt: 0.0,
        height_pt: 0.0,
    }
}

impl PreviewPage {
    pub fn generated(self) -> Result<storage::GeneratedPage, String> {
        let mut objects = Vec::new();
        let mut blocks = Vec::new();
        append(self.items, &mut objects, &mut blocks, &mut Vec::new(), 0)?;
        Ok(storage::GeneratedPage {
            background: self.background,
            objects,
            blocks,
        })
    }

    pub fn insert(self, snapshot: &mut storage::NotebookSnapshot) -> Result<(), String> {
        let last = snapshot
            .page
            .objects
            .iter()
            .map(|obj| obj.fields().z_index)
            .chain(
                snapshot
                    .ink_layers
                    .iter()
                    .flat_map(|layer| layer.strokes.iter().map(|stroke| stroke.z_index)),
            )
            .max()
            .unwrap_or(0);
        append(
            self.items,
            &mut snapshot.page.objects,
            &mut snapshot.blocks,
            &mut snapshot.page.reading_order,
            last,
        )?;
        storage::raise_schema_version(snapshot);
        Ok(())
    }
}

fn append(
    items: Vec<ProposalItem>,
    objects: &mut Vec<PageObject>,
    blocks: &mut Vec<storage::StoredFile>,
    order: &mut Vec<String>,
    mut z: i32,
) -> Result<(), String> {
    for item in items {
        let mut object = item.object;
        if let Some(path) = object.searchable_source() {
            blocks.push(storage::StoredFile {
                path: path.into(),
                bytes: item.source.into_bytes(),
            });
        }
        let fields = object.fields_mut();
        z = z.checked_add(1).ok_or(INVALID)?;
        fields.z_index = z;
        if fields.group_id.is_none() {
            fields.reading_order = order.len().try_into().map_err(|_| INVALID)?;
            order.push(fields.id.clone());
        }
        objects.push(object);
    }
    Ok(())
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    #[test]
    fn addon_add_shape_preserves_caps_and_rejects_invalid_values_and_geometry() {
        let root = tempfile::tempdir().unwrap();
        let geometry: PageGeometry = serde_json::from_value(json!({
            "widthPt": 595, "heightPt": 842
        }))
        .unwrap();
        let mut reply = json!({ "batch": { "operations": [{
            "op": "addShape", "x": 20, "y": 20,
            "geometry": {"kind": "line", "start": {"x": 0, "y": 0}, "end": {"x": 30, "y": 0}},
            "style": {"strokeColor": "#123456", "strokeWidthPt": 2, "fillColor": null,
                "opacity": 1, "startCap": "bar", "endCap": "arrow"}
        }] } });
        let [preview] = prepare(root.path(), &geometry, reply.clone(), false, || false)
            .unwrap()
            .try_into()
            .ok()
            .unwrap();
        let PageObject::Shape { style, .. } = &preview.items[0].object else {
            panic!("expected a shape");
        };
        assert_eq!(
            (style.start_cap, style.end_cap),
            (goodtype_core::ShapeCap::Bar, goodtype_core::ShapeCap::Arrow)
        );
        let mut invalid = reply.clone();
        invalid["batch"]["operations"][0]["style"]["endCap"] = "custom".into();
        assert!(prepare(root.path(), &geometry, invalid, false, || false).is_err());
        reply["batch"]["operations"][0]["geometry"] =
            json!({"kind": "ellipse", "widthPt": 30, "heightPt": 20});
        assert!(prepare(root.path(), &geometry, reply, false, || false).is_err());
    }
}
