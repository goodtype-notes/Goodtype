use super::{selection, *};
use crate::helper::proposals::ProposalItem;
use goodtype_core::{
    ObjectFields, PageBackground, PageGeometry, PageObject, ShapeGeometry, ShapeStyle, storage,
    template,
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
            geometry.validate().map_err(|_| INVALID)?;
            style.validate().map_err(|_| INVALID)?;
            if !fits(page, x, y, &geometry, style.stroke_width_pt / 2.0) {
                return Err("addon-does-not-fit".into());
            }
            Ok(preview_item(PageObject::Shape {
                fields: fresh_fields(x, y)?,
                geometry,
                style,
            }))
        }
        _ => Err(INVALID.into()),
    }
}

fn fits(page: &PageGeometry, x: f64, y: f64, shape: &ShapeGeometry, pad: f64) -> bool {
    let contains = |px, py| {
        (pad..=page.width_pt - pad).contains(&(x + px))
            && (pad..=page.height_pt - pad).contains(&(y + py))
    };
    if !(0.0..=page.width_pt).contains(&x) || !(0.0..=page.height_pt).contains(&y) {
        return false;
    }
    match shape {
        ShapeGeometry::Rectangle {
            width_pt,
            height_pt,
            ..
        }
        | ShapeGeometry::Ellipse {
            width_pt,
            height_pt,
        } => contains(0.0, 0.0) && contains(*width_pt, *height_pt),
        ShapeGeometry::Line { start, end } => contains(start.x, start.y) && contains(end.x, end.y),
        ShapeGeometry::Spline { nodes, .. } => nodes.iter().all(|node| {
            contains(node.point.x, node.point.y)
                && [node.handle_in, node.handle_out]
                    .into_iter()
                    .flatten()
                    .all(|handle| contains(node.point.x + handle.x, node.point.y + handle.y))
        }),
    }
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
        )
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
