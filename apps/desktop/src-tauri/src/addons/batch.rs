use super::{selection, *};
use crate::helper::proposals::ProposalItem;
use goodtype_core::{
    ObjectFields, PageBackground, PageGeometry, PageObject, ShapeGeometry, ShapePoint, ShapeStyle,
    storage, template,
};
use serde_json::{Value, json};

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
    },
    #[serde(rename = "addShape")]
    Shape {
        x: f64,
        y: f64,
        geometry: ShapeGeometry,
        style: ShapeStyle,
    },
    #[serde(rename = "addGroup")]
    Group { children: Vec<Operation> },
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
                    items: objects(root, geometry, ops, &mut budget, &check)?,
                })
            })
            .collect()
    } else {
        if operations.is_empty() {
            return Err(INVALID.into());
        }
        Ok(vec![PreviewPage {
            background: white(),
            items: objects(root, geometry, operations, &mut budget, &check)?,
        }])
    }
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
        if let Operation::Group { children } = op {
            if children.is_empty() {
                return Err(INVALID.into());
            }
            *budget = budget.checked_sub(children.len()).ok_or(INVALID)?;
            let fields = fresh_fields(0.0, 0.0)?;
            let mut child_ids = Vec::new();
            for child in children {
                check()?;
                let mut item = leaf(root, geometry, child)?;
                let child = item.object.fields_mut();
                child.group_id = Some(fields.id.clone());
                child_ids.push(child.id.clone());
                items.push(item);
            }
            items.push(preview_item(PageObject::Group { fields, child_ids }));
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
            selection::compile(
                root,
                page,
                format!("addon-{}", crate::helper::random_hex(16)?),
                op,
            )
        }
        Operation::Shape {
            x,
            y,
            geometry,
            style,
        } => {
            let area = (page.width_pt, page.height_pt, style.stroke_width_pt / 2.0);
            let object = shape_object(fresh_fields(x, y)?, geometry, style).map_err(|_| INVALID)?;
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
    style.validate()
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
fn shape_extent(shape: &ShapeGeometry) -> Option<[f64; 4]> {
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
