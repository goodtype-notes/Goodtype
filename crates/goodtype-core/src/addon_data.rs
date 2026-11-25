//! Addon data and connection ports on ordinary page objects (ADR 0046).
//!
//! The core understands only `refs`: it checks that they name objects on the same page, removes a
//! deleted object from every list in the same commit, and remaps them when objects are copied
//! together. `data` is opaque JSON, bounded so one object cannot grow a page without limit.
//! Rendering and export never read either field.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ObjectFields, PageObject, ShapeGeometry, ShapePoint};

/// At most this many addons may keep data on one object.
pub const MAX_ADDON_ENTRIES: usize = 4;
/// Objects one entry may depend on.
pub const MAX_ADDON_REFS: usize = 32;
/// Serialized size of one entry's `data`.
pub const MAX_ADDON_DATA_BYTES: usize = 16 * 1024;
/// Nesting of arrays and objects inside `data`; a scalar has depth 0.
pub const MAX_ADDON_DATA_DEPTH: usize = 8;
/// Stored connection points on a shape or group.
pub const MAX_PORTS: usize = 32;

/// One addon's entry on an object: the objects it depends on, and data only that addon reads.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddonEntry {
    pub refs: Vec<String>,
    pub data: Value,
}

/// Keyed by addon ID. Unknown IDs are kept: the addon may simply be uninstalled.
pub type AddonData = BTreeMap<String, AddonEntry>;

/// Addon IDs follow the manifest rule: lowercase kebab case, 1–48 characters.
pub fn valid_addon_id(value: &str) -> bool {
    value.len() <= 48
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Value::Object(fields) => 1 + fields.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// Every rule an entry has to meet on its own. `ids` holds every object ID on the page.
pub fn validate_entry(
    owner: &str,
    entry: &AddonEntry,
    ids: &HashSet<&str>,
) -> Result<(), &'static str> {
    if entry.refs.len() > MAX_ADDON_REFS {
        return Err("addon data has too many references");
    }
    let mut seen = HashSet::new();
    for id in &entry.refs {
        if id == owner || !seen.insert(id.as_str()) || !ids.contains(id.as_str()) {
            return Err("addon data references must be unique objects on the same page");
        }
    }
    if depth(&entry.data) > MAX_ADDON_DATA_DEPTH {
        return Err("addon data is nested too deeply");
    }
    let bytes = serde_json::to_vec(&entry.data).map_err(|_| "addon data is not JSON")?;
    if bytes.len() > MAX_ADDON_DATA_BYTES {
        return Err("addon data is too large");
    }
    Ok(())
}

/// The addon data and ports of one object, checked against the page it sits on.
pub fn validate_object(object: &PageObject, ids: &HashSet<&str>) -> Result<(), &'static str> {
    let fields = object.fields();
    if let Some(data) = &fields.addon_data {
        if data.len() > MAX_ADDON_ENTRIES {
            return Err("an object carries data for too many addons");
        }
        for (addon, entry) in data {
            if !valid_addon_id(addon) {
                return Err("addon data keys must be addon IDs");
            }
            validate_entry(&fields.id, entry, ids)?;
        }
    }
    if let Some(ports) = &fields.ports {
        if !matches!(object, PageObject::Shape { .. } | PageObject::Group { .. }) {
            return Err("only shapes and groups carry ports");
        }
        if ports.len() > MAX_PORTS || ports.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err("ports must be at most 32 finite points");
        }
    }
    Ok(())
}

/// Whether an object uses anything introduced by schema 5.
pub fn uses_addon_fields(fields: &ObjectFields) -> bool {
    fields.addon_data.is_some() || fields.ports.is_some()
}

/// Drop `deleted` from every `refs` list on `objects`. Data stays untouched: the addon learns
/// about the deletion through its reaction.
pub fn remove_refs(objects: &mut [PageObject], deleted: &HashSet<&str>) {
    if deleted.is_empty() {
        return;
    }
    for object in objects {
        if let Some(data) = &mut object.fields_mut().addon_data {
            for entry in data.values_mut() {
                entry.refs.retain(|id| !deleted.contains(id.as_str()));
            }
        }
    }
}

/// Refs of a copied object: inside the copied set they follow the copies, outside they keep
/// pointing at the original.
pub fn remap_refs(data: &AddonData, copies: &HashMap<String, String>) -> AddonData {
    data.iter()
        .map(|(addon, entry)| {
            (
                addon.clone(),
                AddonEntry {
                    refs: entry
                        .refs
                        .iter()
                        .map(|id| copies.get(id).cloned().unwrap_or_else(|| id.clone()))
                        .collect(),
                    data: entry.data.clone(),
                },
            )
        })
        .collect()
}

/// Remove one addon's entry; `None` when nothing is left so the field disappears.
pub fn detach(data: Option<AddonData>, addon: &str) -> Option<AddonData> {
    let mut data = data?;
    data.remove(addon);
    (!data.is_empty()).then_some(data)
}

/// The documented automatic ports when an object stores none, in object-local points.
///
/// Stable order: line start, end; rectangle top, right, bottom, left midpoints, then top-left,
/// top-right, bottom-right, bottom-left corners; ellipse top, right, bottom, left; spline first and
/// last node; anything else none.
pub fn automatic_ports(object: &PageObject) -> Vec<ShapePoint> {
    let point = |x, y| ShapePoint { x, y };
    match object {
        PageObject::Shape { geometry, .. } => match geometry {
            ShapeGeometry::Line { start, end } => vec![*start, *end],
            ShapeGeometry::Rectangle {
                width_pt: w,
                height_pt: h,
                ..
            } => vec![
                point(w / 2.0, 0.0),
                point(*w, h / 2.0),
                point(w / 2.0, *h),
                point(0.0, h / 2.0),
                point(0.0, 0.0),
                point(*w, 0.0),
                point(*w, *h),
                point(0.0, *h),
            ],
            ShapeGeometry::Ellipse {
                width_pt: w,
                height_pt: h,
            } => vec![
                point(w / 2.0, 0.0),
                point(*w, h / 2.0),
                point(w / 2.0, *h),
                point(0.0, h / 2.0),
            ],
            ShapeGeometry::Spline { nodes, .. } => match (nodes.first(), nodes.last()) {
                (Some(first), Some(last)) => vec![first.point, last.point],
                _ => vec![],
            },
        },
        _ => vec![],
    }
}

/// An object's ports in object-local points: the stored ones, else the automatic ones.
pub fn local_ports(object: &PageObject) -> Vec<ShapePoint> {
    match &object.fields().ports {
        Some(ports) if matches!(object, PageObject::Shape { .. } | PageObject::Group { .. }) => {
            ports.clone()
        }
        _ => automatic_ports(object),
    }
}

/// A local point on the page: the transform every renderer applies,
/// `translate(x y) rotate(rotation) scale(scale)`. Mirrored by `portPagePoint` in the frontend.
pub fn to_page_point(fields: &ObjectFields, local: ShapePoint) -> ShapePoint {
    let (sin, cos) = fields.rotation.to_radians().sin_cos();
    let x = local.x * fields.scale;
    let y = local.y * fields.scale;
    ShapePoint {
        x: x * cos - y * sin + fields.x,
        y: x * sin + y * cos + fields.y,
    }
}

/// Every port of an object in page points, in port order.
pub fn page_ports(object: &PageObject) -> Vec<ShapePoint> {
    let fields = object.fields();
    local_ports(object)
        .into_iter()
        .map(|local| to_page_point(fields, local))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PortCase {
        name: String,
        object: PageObject,
        ports: Vec<ShapePoint>,
    }

    /// The same file is read by the frontend's port tests, so both implementations agree.
    #[test]
    fn port_positions_match_the_shared_fixture() {
        let cases: Vec<PortCase> = serde_json::from_str(include_str!(
            "../../../fixtures/addon-data/port-positions.json"
        ))
        .unwrap();
        assert!(cases.len() >= 6);
        for case in cases {
            let actual = page_ports(&case.object);
            assert_eq!(actual.len(), case.ports.len(), "{}", case.name);
            for (a, e) in actual.iter().zip(&case.ports) {
                assert!(
                    (a.x - e.x).abs() < 1e-6 && (a.y - e.y).abs() < 1e-6,
                    "{}: {a:?} != {e:?}",
                    case.name
                );
            }
        }
    }

    #[test]
    fn entries_enforce_every_limit() {
        let ids: HashSet<&str> = ["a", "b", "owner"].into_iter().collect();
        let entry = |refs: &[&str], data: Value| AddonEntry {
            refs: refs.iter().map(|id| id.to_string()).collect(),
            data,
        };
        assert!(validate_entry("owner", &entry(&["a", "b"], json!({"k": 1})), &ids).is_ok());
        for bad in [
            entry(&["owner"], json!(null)),
            entry(&["a", "a"], json!(null)),
            entry(&["missing"], json!(null)),
            entry(&[], json!("x".repeat(MAX_ADDON_DATA_BYTES))),
            entry(&[], json!([[[[[[[[[1]]]]]]]]])),
        ] {
            assert!(validate_entry("owner", &bad, &ids).is_err(), "{bad:?}");
        }
        assert!(validate_entry("owner", &entry(&[], json!([[[[[[[[1]]]]]]]])), &ids).is_ok());
        let many: Vec<String> = (0..33).map(|i| format!("o{i}")).collect();
        let all: HashSet<&str> = many.iter().map(String::as_str).collect();
        let too_many = AddonEntry {
            refs: many.clone(),
            data: json!(null),
        };
        assert!(validate_entry("owner", &too_many, &all).is_err());
        assert!(valid_addon_id("connectors") && !valid_addon_id("Connectors"));
        assert!(!valid_addon_id(&"a".repeat(49)) && !valid_addon_id("a--b"));
    }

    #[test]
    fn copies_follow_refs_inside_the_set_and_keep_the_rest() {
        let data = AddonData::from([(
            "connectors".into(),
            AddonEntry {
                refs: vec!["inside".into(), "outside".into()],
                data: json!({"routing": "orthogonal"}),
            },
        )]);
        let copies = HashMap::from([("inside".to_string(), "copy".to_string())]);
        let remapped = remap_refs(&data, &copies);
        assert_eq!(remapped["connectors"].refs, ["copy", "outside"]);
        assert_eq!(remapped["connectors"].data, data["connectors"].data);
        assert_eq!(detach(Some(data), "connectors"), None);
    }
}
