//! Invocation-local geometry reads. Never serialize canonical objects on this boundary.
use super::{Invocation, PageObject, object_snapshot, storage};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

pub(in crate::addons) const FEATURE: &str = "scoped-reads-v0";
const MAX_OBJECTS: usize = 256;
const MAX_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 256 * 1024;
const MAX_CALLS: usize = 16;
const MARGIN: f64 = 144.0;
const MAX_SIDE: f64 = 1024.0;

pub(in crate::addons) struct Reads {
    snapshot: storage::NotebookSnapshot,
    groups: HashSet<String>,
    region: [f64; 4],
    calls: usize,
    bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GroupRequest {
    revision: u64,
    group_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionRequest {
    revision: u64,
    bounds: Bounds,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Reads {
    pub(in crate::addons) fn capture(
        snapshot: &storage::NotebookSnapshot,
        invocation: &Invocation,
    ) -> Option<Self> {
        let values: Vec<&Value> = match invocation {
            Invocation::OwnObjects(objects) => objects.iter().collect(),
            Invocation::Tool { params, .. }
            | Invocation::PathEdit(params)
            | Invocation::Reaction(params) => ["objects", "refs", "own", "changed", "points"]
                .into_iter()
                .flat_map(|key| params[key].as_array().into_iter().flatten())
                .chain(params.get("object"))
                .collect(),
            _ => return None,
        };
        let ids: HashSet<_> = values
            .iter()
            .filter_map(|value| value["id"].as_str())
            .collect();
        let groups = snapshot
            .page
            .objects
            .iter()
            .filter(|object| {
                matches!(object, PageObject::Group { .. })
                    && ids.contains(object.fields().id.as_str())
            })
            .map(|object| object.fields().id.clone())
            .collect();
        let mut region = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for object in &snapshot.page.objects {
            if ids.contains(object.fields().id.as_str()) {
                let [l, t, r, b] = super::object_extent(object, snapshot);
                region = [
                    region[0].min(l),
                    region[1].min(t),
                    region[2].max(r),
                    region[3].max(b),
                ];
            }
        }
        for value in values {
            if value.get("id").is_none()
                && let (Some(x), Some(y)) = (value["x"].as_f64(), value["y"].as_f64())
            {
                region = [
                    region[0].min(x),
                    region[1].min(y),
                    region[2].max(x),
                    region[3].max(y),
                ];
            }
        }
        region = [
            (region[0] - MARGIN).max(0.0),
            (region[1] - MARGIN).max(0.0),
            (region[2] + MARGIN).min(snapshot.page.geometry.width_pt),
            (region[3] + MARGIN).min(snapshot.page.geometry.height_pt),
        ];
        if !region.iter().all(|v| v.is_finite()) || region[0] > region[2] || region[1] > region[3] {
            region = [0.0; 4];
        }
        Some(Self {
            snapshot: snapshot.clone(),
            groups,
            region,
            calls: 0,
            bytes: 0,
        })
    }

    pub(in crate::addons) fn offer(&self) -> Value {
        let [x, y, r, b] = self.region;
        let mut groups: Vec<_> = self.groups.iter().collect();
        groups.sort();
        json!({"revision": self.snapshot.page.revision, "groups": groups, "region": {"x":x,"y":y,"width":(r-x).max(0.0),"height":(b-y).max(0.0)},
        "maxObjects":MAX_OBJECTS,"maxBytes":MAX_BYTES,"maxCalls":MAX_CALLS,"maxTotalBytes":MAX_TOTAL_BYTES,"maxRegionSidePt":MAX_SIDE})
    }

    pub(in crate::addons) fn reply(&mut self, frame: Value) -> Option<Value> {
        let fields = frame.as_object()?;
        let id = fields.get("id")?.as_str()?;
        if id.is_empty() || id.len() > 80 || fields.len() != 4 || fields.get("jsonrpc")? != "2.0" {
            return None;
        }
        let method = fields.get("method")?.as_str()?;
        if !["group/descendants", "region/obstacles"].contains(&method) {
            return None;
        }
        self.calls = self.calls.saturating_add(1);
        let result = if self.calls > MAX_CALLS {
            Err("limit-exceeded")
        } else {
            self.read(method, fields.get("params")?.clone())
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(reason) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32004,"message":reason}})
            }
        })
    }

    fn read(&mut self, method: &str, params: Value) -> Result<Value, &'static str> {
        let mut objects = Vec::new();
        if method == "group/descendants" {
            let request: GroupRequest =
                serde_json::from_value(params).map_err(|_| "invalid-params")?;
            self.revision(request.revision)?;
            if !self.groups.contains(&request.group_id) {
                return Err("out-of-scope");
            }
            let mut pending = vec![request.group_id];
            let mut seen = HashSet::new();
            while let Some(id) = pending.pop() {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let object = self
                    .snapshot
                    .page
                    .objects
                    .iter()
                    .find(|object| object.fields().id == id)
                    .ok_or("out-of-scope")?;
                if let PageObject::Group { child_ids, .. } = object {
                    for id in child_ids {
                        let child = self
                            .snapshot
                            .page
                            .objects
                            .iter()
                            .find(|object| object.fields().id == *id)
                            .ok_or("out-of-scope")?;
                        let mut value = object_snapshot(child, &self.snapshot);
                        value["parentId"] = json!(object.fields().id);
                        objects.push(value);
                        if objects.len() > MAX_OBJECTS {
                            return Err("limit-exceeded");
                        }
                        pending.push(id.clone());
                    }
                }
            }
        } else {
            let request: RegionRequest =
                serde_json::from_value(params).map_err(|_| "invalid-params")?;
            self.revision(request.revision)?;
            let Bounds {
                x,
                y,
                width,
                height,
            } = request.bounds;
            let [l, t, r, b] = self.region;
            if ![x, y, width, height].iter().all(|v| v.is_finite())
                || width <= 0.0
                || height <= 0.0
                || width > MAX_SIDE
                || height > MAX_SIDE
                || x < l
                || y < t
                || x + width > r
                || y + height > b
            {
                return Err("out-of-scope");
            }
            for object in &self.snapshot.page.objects {
                let [l, t, r, b] = super::object_extent(object, &self.snapshot);
                if l <= x + width && r >= x && t <= y + height && b >= y {
                    let mut value = object_snapshot(object, &self.snapshot);
                    value
                        .as_object_mut()
                        .unwrap()
                        .retain(|key, _| ["id", "type", "bounds", "ports"].contains(&key.as_str()));
                    objects.push(value);
                    if objects.len() > MAX_OBJECTS {
                        return Err("limit-exceeded");
                    }
                }
            }
        }
        let result = json!({"revision":self.snapshot.page.revision,"objects":objects});
        let bytes = serde_json::to_vec(&result)
            .map_err(|_| "limit-exceeded")?
            .len();
        if bytes > MAX_BYTES || self.bytes + bytes > MAX_TOTAL_BYTES {
            return Err("limit-exceeded");
        }
        self.bytes += bytes;
        Ok(result)
    }

    fn revision(&self, revision: u64) -> Result<(), &'static str> {
        if revision != self.snapshot.page.revision {
            Err("revision-mismatch")
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
pub(in crate::addons) fn fixture() -> (storage::NotebookSnapshot, Reads) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/notebooks/minimal");
    let mut snapshot = storage::open_notebook(&root).unwrap();
    snapshot.page.revision = 7;
    let child: PageObject = serde_json::from_value(json!({
        "id":"child","type":"shape","x":30.0,"y":40.0,"rotation":0.0,"scale":2.0,
        "zIndex":1,"readingOrder":0,"groupId":"symbol","createdAt":"2026-11-23","modifiedAt":"2026-11-23",
        "ports":[{"x":0.0,"y":5.0}],
        "addonData":{"another":{"refs":[],"data":{"secret":"private"}}},
        "geometry":{"kind":"rectangle","widthPt":20.0,"heightPt":10.0,"cornerRadiusPt":0.0},
        "style":{"strokeColor":"#000000","strokeWidthPt":1.0,"fillColor":null,"opacity":1.0}
    })).unwrap();
    let mut fields = child.fields().clone();
    fields.id = "symbol".into();
    fields.group_id = None;
    let group = PageObject::Group {
        fields,
        child_ids: vec!["child".into()],
        stretch: None,
    };
    let mut unrelated = child.clone();
    unrelated.fields_mut().id = "unrelated".into();
    unrelated.fields_mut().group_id = None;
    snapshot.page.objects = vec![group, child, unrelated];
    let reads = Reads::capture(
        &snapshot,
        &Invocation::OwnObjects(vec![json!({"id":"symbol"})]),
    )
    .unwrap();
    (snapshot, reads)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> Value {
        json!({"revision":7,"bounds":{"x":20,"y":20,"width":100,"height":100}})
    }

    #[test]
    fn scoped_group_geometry_and_private_fields() {
        let (_, mut reads) = fixture();
        let result = reads
            .read(
                "group/descendants",
                json!({"revision":7,"groupId":"symbol"}),
            )
            .unwrap();
        assert_eq!(result["objects"].as_array().unwrap().len(), 1);
        let child = &result["objects"][0];
        assert_eq!(child["id"], "child");
        assert_eq!(child["parentId"], "symbol");
        assert_eq!(
            child["bounds"],
            json!({"x":30.0,"y":40.0,"width":40.0,"height":20.0})
        );
        assert_eq!(child["ports"][0], json!({"x":30.0,"y":50.0}));
        assert!(child.get("geometry").is_some());
        for key in [
            "addonData",
            "style",
            "source",
            "sourcePath",
            "text",
            "points",
            "asset",
            "createdAt",
        ] {
            assert!(child.get(key).is_none(), "{key}");
        }
        assert_eq!(
            reads.read(
                "group/descendants",
                json!({"revision":7,"groupId":"unrelated"})
            ),
            Err("out-of-scope")
        );
        assert_eq!(
            reads.read(
                "group/descendants",
                json!({"revision":8,"groupId":"symbol"})
            ),
            Err("revision-mismatch")
        );
    }

    #[test]
    fn obstacles_are_allowlisted_and_captured_at_one_revision() {
        let (mut snapshot, mut reads) = fixture();
        snapshot.page.revision = 8;
        snapshot.page.objects.clear();
        let result = reads.read("region/obstacles", region()).unwrap();
        assert_eq!(result["revision"], 7);
        assert_eq!(result["objects"].as_array().unwrap().len(), 3);
        for object in result["objects"].as_array().unwrap() {
            assert_eq!(object.as_object().unwrap().len(), 4);
            for key in ["id", "type", "bounds", "ports"] {
                assert!(object.get(key).is_some());
            }
        }
        let mut stale = region();
        stale["revision"] = json!(8);
        assert_eq!(
            reads.read("region/obstacles", stale),
            Err("revision-mismatch")
        );
        let mut outside = region();
        outside["bounds"]["x"] = json!(500);
        assert_eq!(reads.read("region/obstacles", outside), Err("out-of-scope"));
        let mut oversized = region();
        oversized["bounds"]["width"] = json!(1025);
        assert_eq!(
            reads.read("region/obstacles", oversized),
            Err("out-of-scope")
        );
        let mut unknown = region();
        unknown["source"] = json!(true);
        assert_eq!(
            reads.read("region/obstacles", unknown),
            Err("invalid-params")
        );
    }

    #[test]
    fn count_bytes_and_invocation_budgets_fail_closed() {
        let (_, mut reads) = fixture();
        let child = reads.snapshot.page.objects[1].clone();
        for i in 0..MAX_OBJECTS {
            let mut object = child.clone();
            object.fields_mut().id = format!("extra-{i}");
            reads.snapshot.page.objects.push(object);
        }
        assert_eq!(
            reads.read("region/obstacles", region()),
            Err("limit-exceeded")
        );
        if let PageObject::Group { child_ids, .. } = &mut reads.snapshot.page.objects[0] {
            child_ids.extend((0..MAX_OBJECTS).map(|i| format!("extra-{i}")));
        }
        assert_eq!(
            reads.read(
                "group/descendants",
                json!({"revision":7,"groupId":"symbol"})
            ),
            Err("limit-exceeded")
        );
        let (_, mut reads) = fixture();
        reads.snapshot.page.objects[1].fields_mut().id = "x".repeat(MAX_BYTES);
        assert_eq!(
            reads.read("region/obstacles", region()),
            Err("limit-exceeded")
        );
        let (_, mut reads) = fixture();
        reads.bytes = MAX_TOTAL_BYTES;
        assert_eq!(
            reads.read("region/obstacles", region()),
            Err("limit-exceeded")
        );
        let (_, mut reads) = fixture();
        for _ in 0..MAX_CALLS {
            assert!(reads.reply(json!({"jsonrpc":"2.0","id":"read-1","method":"region/obstacles","params":region()})).unwrap().get("result").is_some());
        }
        assert_eq!(reads.reply(json!({"jsonrpc":"2.0","id":"read-1","method":"region/obstacles","params":region()})).unwrap()["error"]["message"], "limit-exceeded");
    }

    #[test]
    fn nested_groups_follow_only_explicit_children() {
        let (mut snapshot, _) = fixture();
        let mut inner = snapshot.page.objects[0].clone();
        inner.fields_mut().id = "nested".into();
        inner.fields_mut().group_id = Some("symbol".into());
        if let PageObject::Group { child_ids, .. } = &mut snapshot.page.objects[0] {
            *child_ids = vec!["nested".into()];
        }
        snapshot.page.objects.push(inner);
        let mut reads = Reads::capture(
            &snapshot,
            &Invocation::OwnObjects(vec![json!({"id":"symbol"})]),
        )
        .unwrap();
        let result = reads
            .read(
                "group/descendants",
                json!({"revision":7,"groupId":"symbol"}),
            )
            .unwrap();
        assert_eq!(result["objects"][0]["id"], "nested");
        assert_eq!(result["objects"][1]["parentId"], "nested");
        assert_eq!(
            reads.read(
                "group/descendants",
                json!({"revision":7,"groupId":"nested"})
            ),
            Err("out-of-scope")
        );
    }
}

#[cfg(test)]
mod conformance {
    use super::*;
    #[test]
    fn shared_wire_cases() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../../fixtures/addon-scoped-reads/requests.json"
        ))
        .unwrap();
        for case in cases {
            let (_, mut reads) = fixture();
            let result = reads.read(case["method"].as_str().unwrap(), case["params"].clone());
            if let Some(error) = case["error"].as_str() {
                assert_eq!(result, Err(error), "{case}");
            } else {
                let result = result.unwrap();
                let ids: Vec<_> = result["objects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|object| object["id"].clone())
                    .collect();
                assert_eq!(json!(ids), case["ids"], "{case}");
            }
        }
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    #[test]
    fn tools_paths_and_reaction_chunks_keep_their_admitted_groups() {
        let (mut snapshot, _) = fixture();
        let mut other = snapshot.page.objects[0].clone();
        other.fields_mut().id = "other-group".into();
        snapshot.page.objects.push(other);
        for invocation in [
            Invocation::Tool {
                points: 2,
                snapped: true,
                params: json!({"objects":[{"id":"symbol"}],"points":[{"x":50,"y":50},{"x":80,"y":50}]}),
            },
            Invocation::PathEdit(json!({"object":{"id":"child"},"refs":[{"id":"symbol"}]})),
            Invocation::Reaction(json!({"own":[{"id":"child"}],"changed":[{"id":"symbol"}]})),
        ] {
            let reads = Reads::capture(&snapshot, &invocation).unwrap();
            assert_eq!(reads.offer()["groups"], json!(["symbol"]));
            assert!(reads.offer()["region"]["width"].as_f64().unwrap() > 0.0);
        }
        let reads = Reads::capture(&snapshot, &Invocation::OwnObjects(vec![])).unwrap();
        assert_eq!(reads.offer()["region"]["width"], 0.0);
        assert!(Reads::capture(&snapshot, &Invocation::Selection(json!({}))).is_none());
    }
}
