//! Storage behaviour of addon data and ports (ADR 0046): schema 5, limits, refs cleanup and page
//! duplication.

use serde_json::json;

use super::{fixtures::*, *};
use crate::{
    ADDON_DATA_SCHEMA_VERSION, PageObject, ShapeGeometry, ShapePoint, ShapeStyle, StrokePattern,
    addon_data::{AddonData, AddonEntry},
};

fn shape(id: &str, order: u32) -> PageObject {
    PageObject::Shape {
        fields: fields(id, order),
        geometry: ShapeGeometry::Rectangle {
            width_pt: 40.0,
            height_pt: 20.0,
            corner_radius_pt: 0.0,
        },
        style: ShapeStyle {
            stroke_color: "#16212b".into(),
            stroke_width_pt: 1.5,
            fill_color: None,
            opacity: 1.0,
            pattern: StrokePattern::Solid,
            pattern_spacing: 1.0,
            pattern_offset_pt: 0.0,
        },
    }
}

fn entry(refs: &[&str]) -> AddonData {
    AddonData::from([(
        "connectors".into(),
        AddonEntry {
            refs: refs.iter().map(|id| id.to_string()).collect(),
            data: json!({"routing": "orthogonal", "bends": [12.5], "start": {"port": 1}}),
        },
    )])
}

/// Two parts and a connector that depends on both.
fn connected() -> NotebookSnapshot {
    let mut snapshot = snapshot();
    snapshot.page.objects.push(shape("part-a", 2));
    snapshot.page.objects.push(shape("part-b", 3));
    let mut connector = shape("connector", 4);
    connector.fields_mut().addon_data = Some(entry(&["part-a", "part-b"]));
    snapshot.page.objects.push(connector);
    snapshot
        .page
        .reading_order
        .extend(["part-a".into(), "part-b".into(), "connector".into()]);
    if let PageObject::Shape { fields, .. } = &mut snapshot.page.objects[2] {
        fields.ports = Some(vec![ShapePoint { x: 0.0, y: 8.0 }]);
    }
    snapshot
}

fn object<'a>(snapshot: &'a NotebookSnapshot, id: &str) -> &'a PageObject {
    snapshot
        .page
        .objects
        .iter()
        .find(|object| object.fields().id == id)
        .unwrap()
}

fn committed(root: &std::path::Path, change: impl FnOnce(&mut NotebookSnapshot)) -> HistoryResult {
    let mut history = NotebookHistory::default();
    let mut candidate = observe_notebook(root, &mut history).unwrap();
    change(&mut candidate);
    commit_notebook(root, &mut history, candidate).unwrap()
}

#[test]
fn schema_four_pages_round_trip_untouched() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("notebook");
    create_notebook(&root, &snapshot()).unwrap();
    let before = std::fs::read(root.join("pages/page-001.json")).unwrap();
    let result = committed(&root, |page| {
        if let PageObject::Image { fields, .. } = &mut page.page.objects[1] {
            fields.x = 100.0;
        }
    });
    assert!(result.snapshot.page.schema_version < ADDON_DATA_SCHEMA_VERSION);
    let after =
        String::from_utf8(std::fs::read(root.join("pages/page-001.json")).unwrap()).unwrap();
    assert!(!after.contains("addonData") && !after.contains("ports"));
    assert_ne!(before, after.into_bytes());
}

#[test]
fn addon_data_raises_the_page_to_schema_five_and_round_trips() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("notebook");
    create_notebook(&root, &snapshot()).unwrap();
    let result = committed(&root, |page| *page = connected_on(page));
    assert_eq!(
        result.snapshot.page.schema_version,
        ADDON_DATA_SCHEMA_VERSION
    );
    assert_eq!(
        result.snapshot.manifest.schema_version,
        ADDON_DATA_SCHEMA_VERSION
    );
    let reopened = open_notebook(&root).unwrap();
    let fields = object(&reopened, "connector").fields();
    assert_eq!(fields.addon_data, Some(entry(&["part-a", "part-b"])));
    assert_eq!(
        object(&reopened, "part-a").fields().ports,
        Some(vec![ShapePoint { x: 0.0, y: 8.0 }])
    );
    // An older build reads at most schema 4 and refuses this notebook.
    let text = std::fs::read_to_string(root.join("pages/page-001.json")).unwrap();
    assert!(text.contains(r#""schemaVersion": 5"#) || text.contains(r#""schemaVersion":5"#));
}

/// The connected fixture rebased onto a freshly observed snapshot (revision and ink paths).
fn connected_on(current: &NotebookSnapshot) -> NotebookSnapshot {
    let mut next = connected();
    next.manifest = current.manifest.clone();
    next.page.revision = current.page.revision;
    next.page.ink_layers = current.page.ink_layers.clone();
    next.ink_layers = current.ink_layers.clone();
    // The frontend states the version its content needs, as it does for shapes.
    raise_schema_version(&mut next);
    next
}

#[test]
fn every_limit_is_enforced_by_validation() {
    let invalid = |change: &dyn Fn(&mut NotebookSnapshot)| {
        let mut candidate = connected();
        raise_schema_version(&mut candidate);
        change(&mut candidate);
        super::validate::validate_snapshot_content(&candidate).is_err()
    };
    assert!(!invalid(&|_| {}));
    let set = |id: &'static str, data: Option<AddonData>| {
        move |candidate: &mut NotebookSnapshot| {
            candidate
                .page
                .objects
                .iter_mut()
                .find(|object| object.fields().id == id)
                .unwrap()
                .fields_mut()
                .addon_data = data.clone();
        }
    };
    assert!(invalid(&set("connector", Some(entry(&["missing"])))));
    assert!(invalid(&set("connector", Some(entry(&["connector"])))));
    assert!(invalid(&set(
        "connector",
        Some(entry(&["part-a", "part-a"]))
    )));
    let five: AddonData = ["a", "b", "c", "d", "e"]
        .into_iter()
        .map(|id| {
            (
                id.to_string(),
                AddonEntry {
                    refs: vec![],
                    data: json!(null),
                },
            )
        })
        .collect();
    assert!(invalid(&set("connector", Some(five))));
    let mut bad_key = entry(&[]);
    bad_key.insert(
        "Not An Id".into(),
        AddonEntry {
            refs: vec![],
            data: json!(null),
        },
    );
    assert!(invalid(&set("connector", Some(bad_key))));
    let big = AddonData::from([(
        "connectors".into(),
        AddonEntry {
            refs: vec![],
            data: json!("x".repeat(16 * 1024)),
        },
    )]);
    assert!(invalid(&set("connector", Some(big))));
    // Ports only on shapes and groups, at most 32, finite.
    assert!(invalid(&|candidate| {
        candidate.page.objects[0].fields_mut().ports = Some(vec![ShapePoint { x: 0.0, y: 0.0 }]);
    }));
    assert!(invalid(&|candidate| {
        candidate.page.objects[2].fields_mut().ports =
            Some(vec![ShapePoint { x: 0.0, y: 0.0 }; 33]);
    }));
    assert!(invalid(&|candidate| {
        candidate.page.objects[2].fields_mut().ports = Some(vec![ShapePoint {
            x: f64::NAN,
            y: 0.0,
        }]);
    }));
    // A page carrying addon data at an older version is refused, never silently upgraded.
    assert!(invalid(&|candidate| {
        candidate.page.schema_version = crate::CALCULATION_SCHEMA_VERSION;
        for layer in &mut candidate.ink_layers {
            layer.schema_version = crate::CALCULATION_SCHEMA_VERSION;
        }
    }));
}

#[test]
fn deleting_a_referenced_object_removes_it_from_refs_in_the_same_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("notebook");
    create_notebook(&root, &snapshot()).unwrap();
    committed(&root, |page| *page = connected_on(page));
    let result = committed(&root, |page| {
        page.page
            .objects
            .retain(|object| object.fields().id != "part-a");
        page.page.reading_order.retain(|id| id != "part-a");
        for (index, id) in page.page.reading_order.clone().iter().enumerate() {
            page.page
                .objects
                .iter_mut()
                .find(|object| &object.fields().id == id)
                .unwrap()
                .fields_mut()
                .reading_order = index as u32;
        }
    });
    let connector = object(&result.snapshot, "connector").fields();
    let entry = &connector.addon_data.as_ref().unwrap()["connectors"];
    assert_eq!(entry.refs, ["part-b"]);
    assert_eq!(entry.data["start"]["port"], 1, "data stays untouched");
}

#[test]
fn a_dangling_ref_that_was_never_deleted_is_still_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("notebook");
    create_notebook(&root, &snapshot()).unwrap();
    let mut history = NotebookHistory::default();
    let current = observe_notebook(&root, &mut history).unwrap();
    let mut candidate = connected_on(&current);
    candidate.page.objects[4].fields_mut().addon_data = Some(entry(&["ghost"]));
    assert!(commit_notebook(&root, &mut history, candidate).is_err());
}

#[test]
fn duplicating_a_page_remaps_every_ref_to_the_copies() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("notebook");
    create_notebook(&root, &snapshot()).unwrap();
    committed(&root, |page| *page = connected_on(page));
    let copy = duplicate_page(&root, "page-001", "2026-11-23T10:00:00Z").unwrap();
    let copied = open_page(&root, &copy.manifest.pages[1].id).unwrap();
    let ids: std::collections::HashSet<_> = copied
        .page
        .objects
        .iter()
        .map(|object| object.fields().id.clone())
        .collect();
    let with_data = copied
        .page
        .objects
        .iter()
        .find_map(|object| object.fields().addon_data.as_ref())
        .unwrap();
    let refs = &with_data["connectors"].refs;
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().all(|id| ids.contains(id)), "{refs:?}");
    assert!(!refs.contains(&"part-a".to_string()));
    assert_eq!(copied.page.schema_version, ADDON_DATA_SCHEMA_VERSION);
}

#[test]
fn an_amendment_joins_the_step_it_reacts_to_for_undo_and_redo() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("notebook");
    create_notebook(&root, &snapshot()).unwrap();
    let mut history = NotebookHistory::default();
    let base = observe_notebook(&root, &mut history).unwrap();
    commit_notebook(&root, &mut history, connected_on(&base)).unwrap();
    // The user moves a part: one undo step.
    let mut moved = open_notebook(&root).unwrap();
    moved.page.objects[2].fields_mut().x = 300.0;
    let moved = commit_notebook(&root, &mut history, moved)
        .unwrap()
        .snapshot;
    // The reaction re-routes the connector inside that same step.
    let mut amended = moved.clone();
    amended.page.objects[4].fields_mut().addon_data = Some(AddonData::from([(
        "connectors".into(),
        AddonEntry {
            refs: vec!["part-a".into(), "part-b".into()],
            data: json!({"routed": true}),
        },
    )]));
    let result = amend_notebook(&root, &mut history, amended).unwrap();
    assert_eq!(result.snapshot.page.revision, moved.page.revision + 1);
    let routed = |snapshot: &NotebookSnapshot| {
        object(snapshot, "connector")
            .fields()
            .addon_data
            .as_ref()
            .unwrap()["connectors"]
            .data
            == json!({"routed": true})
    };
    assert!(routed(&result.snapshot));
    let undone = undo_notebook(&root, &mut history).unwrap().snapshot;
    assert_eq!(object(&undone, "part-a").fields().x, 72.0);
    assert!(!routed(&undone));
    let redone = redo_notebook(&root, &mut history).unwrap().snapshot;
    assert_eq!(object(&redone, "part-a").fields().x, 300.0);
    assert!(routed(&redone));
    // Once something was undone the step is no longer the newest one: no amendment.
    undo_notebook(&root, &mut history).unwrap();
    let stale = open_notebook(&root).unwrap();
    assert!(amend_notebook(&root, &mut history, stale).is_err());
}
