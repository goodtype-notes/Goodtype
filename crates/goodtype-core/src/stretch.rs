use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::addon_data::to_page_point;
use crate::{MAX_PAGE_DIMENSION_PT, ObjectFields, PageObject, ShapeGeometry, ShapePoint as Point};

const EPS: f64 = 1e-6;
const INVALID: &str = "invalid group stretch rule";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Body,
    StartLead,
    EndLead,
    ThroughLine,
    Span,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Part {
    pub child_id: String,
    pub role: Role,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Limits {
    FixedBody {
        socket_separation: f64,
        lead_floors: [f64; 2],
        start_share: f64,
    },
    Span {
        minimum_length: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StretchRule {
    pub axis: [usize; 2],
    pub parts: Vec<Part>,
    pub limits: Limits,
}

fn point(x: f64, y: f64) -> Point {
    Point { x, y }
}
fn add(a: Point, b: Point) -> Point {
    point(a.x + b.x, a.y + b.y)
}
fn sub(a: Point, b: Point) -> Point {
    point(a.x - b.x, a.y - b.y)
}
fn mul(a: Point, k: f64) -> Point {
    point(a.x * k, a.y * k)
}

fn rotate(p: Point, degrees: f64) -> Point {
    let (sin, cos) = degrees.to_radians().sin_cos();
    point(p.x * cos - p.y * sin, p.x * sin + p.y * cos)
}

fn local(fields: &ObjectFields, p: Point) -> Point {
    mul(
        rotate(sub(p, point(fields.x, fields.y)), -fields.rotation),
        1.0 / fields.scale,
    )
}

fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS
}

fn bounded(p: Point) -> bool {
    p.x.abs() <= MAX_PAGE_DIMENSION_PT && p.y.abs() <= MAX_PAGE_DIMENSION_PT
}

pub fn ordered_ports(ports: &[Point], axis: [usize; 2]) -> Result<[Point; 2], &'static str> {
    if let (2, [0, 1] | [1, 0]) = (ports.len(), axis) {
        let [start, end] = axis.map(|index| ports[index]);
        if bounded(start)
            && bounded(end)
            && start.y == end.y
            && end.x - start.x > EPS
            && end.x - start.x <= MAX_PAGE_DIMENSION_PT
        {
            return Ok([start, end]);
        }
    }
    Err(INVALID)
}

fn endpoints(geometry: &ShapeGeometry) -> Option<[Point; 2]> {
    match geometry {
        ShapeGeometry::Line { start, end } => Some([*start, *end]),
        ShapeGeometry::Spline {
            nodes,
            closed: false,
        } => Some([nodes.first()?.point, nodes.last()?.point]),
        ShapeGeometry::Rectangle {
            width_pt,
            height_pt,
            corner_radius_pt,
        } if *corner_radius_pt == 0.0 => Some([
            point(0.0, height_pt / 2.0),
            point(*width_pt, height_pt / 2.0),
        ]),
        _ => None,
    }
}

pub fn inner_lead(ends: [Point; 2], port: Point) -> Result<Point, &'static str> {
    let [a, b] = ends;
    if close(a, port) {
        Ok(b)
    } else if close(b, port) {
        Ok(a)
    } else {
        Err(INVALID)
    }
}

impl Limits {
    pub fn minimum(&self) -> f64 {
        match self {
            Self::FixedBody {
                socket_separation,
                lead_floors,
                ..
            } => socket_separation + lead_floors.iter().sum::<f64>(),
            Self::Span { minimum_length } => *minimum_length,
        }
    }

    pub fn fixed(
        length: f64,
        sockets: [f64; 2],
        lead_floors: [f64; 2],
    ) -> Result<Self, &'static str> {
        let [a, b] = sockets;
        let [floor_a, floor_b] = lead_floors;
        if !(floor_a > 0.0
            && floor_b > 0.0
            && a >= floor_a - EPS
            && b > a
            && length - b >= floor_b - EPS)
        {
            return Err(INVALID);
        }
        let excess = length - (b - a) - floor_a - floor_b;
        Ok(Self::FixedBody {
            socket_separation: b - a,
            lead_floors,
            start_share: if excess <= EPS {
                0.5
            } else {
                ((a - floor_a) / excess).clamp(0.0, 1.0)
            },
        })
    }

    fn sockets(&self, length: f64) -> [f64; 2] {
        let Self::FixedBody {
            socket_separation,
            lead_floors,
            start_share,
        } = self
        else {
            unreachable!()
        };
        let start = lead_floors[0] + (length - self.minimum()).max(0.0) * start_share;
        [start, start + socket_separation]
    }
}

fn bounded_geometry(fields: &ObjectFields, geometry: &ShapeGeometry) -> bool {
    let check = |p| bounded(p) && bounded(to_page_point(fields, p));
    geometry.validate().is_ok()
        && bounded(point(fields.x, fields.y))
        && match geometry {
            ShapeGeometry::Line { start, end } => check(*start) && check(*end),
            ShapeGeometry::Spline { nodes, .. } => nodes.iter().all(|node| {
                check(node.point)
                    && [node.handle_in, node.handle_out]
                        .into_iter()
                        .flatten()
                        .all(|handle| check(add(node.point, handle)))
            }),
            ShapeGeometry::Rectangle {
                width_pt,
                height_pt,
                ..
            }
            | ShapeGeometry::Ellipse {
                width_pt,
                height_pt,
            } => [0.0, *width_pt]
                .into_iter()
                .all(|x| [0.0, *height_pt].into_iter().all(|y| check(point(x, y)))),
        }
}

impl StretchRule {
    pub fn validate(
        &self,
        fields: &ObjectFields,
        child_ids: &[String],
        objects: &HashMap<&str, &PageObject>,
    ) -> Result<(), &'static str> {
        let [start, end] = ordered_ports(fields.ports.as_deref().ok_or(INVALID)?, self.axis)?;
        let length = end.x - start.x;
        let minimum = self.limits.minimum();
        if !(minimum > 0.0
            && minimum <= length + EPS
            && fields.scale > 0.0
            && length * fields.scale <= MAX_PAGE_DIMENSION_PT
            && bounded(to_page_point(fields, start))
            && bounded(to_page_point(fields, end)))
            || !(1..=64).contains(&self.parts.len())
            || self.parts.len() != child_ids.len()
        {
            return Err(INVALID);
        }
        let ids: HashSet<_> = child_ids.iter().map(String::as_str).collect();
        let mut seen = HashSet::new();
        let mut count = [0; 5];
        let mut covers_axis = false;
        for part in &self.parts {
            let id = part.child_id.as_str();
            if !ids.contains(id) || !seen.insert(id) {
                return Err(INVALID);
            }
            let Some(PageObject::Shape {
                fields: child,
                geometry,
                ..
            }) = objects.get(id)
            else {
                return Err(INVALID);
            };
            if child.group_id.as_deref() != Some(&fields.id)
                || !(child.scale > 0.0 && child.scale.is_finite())
                || !bounded_geometry(child, geometry)
            {
                return Err(INVALID);
            }
            let ends = || {
                endpoints(geometry)
                    .ok_or(INVALID)
                    .map(|ends| ends.map(|p| local(fields, to_page_point(child, p))))
            };
            count[part.role as usize] += 1;
            match (&self.limits, part.role) {
                (Limits::FixedBody { .. }, Role::Body) => {}
                (Limits::FixedBody { .. }, Role::StartLead | Role::EndLead | Role::ThroughLine) => {
                    if !matches!(geometry, ShapeGeometry::Line { .. }) {
                        return Err(INVALID);
                    }
                    let ends = ends()?;
                    if part.role == Role::ThroughLine {
                        if !close(inner_lead(ends, start)?, end) {
                            return Err(INVALID);
                        }
                    } else {
                        let side = usize::from(part.role == Role::EndLead);
                        let socket = point(start.x + self.limits.sockets(length)[side], start.y);
                        if !close(inner_lead(ends, [start, end][side])?, socket) {
                            return Err(INVALID);
                        }
                    }
                }
                (Limits::Span { .. }, Role::Span) => {
                    let [a, b] = ends()?;
                    if matches!(geometry, ShapeGeometry::Rectangle { .. })
                        && ((a.y - b.y).abs() > EPS || (a.x - b.x).abs() <= EPS)
                    {
                        return Err(INVALID);
                    }
                    covers_axis |=
                        (close(a, start) && close(b, end)) || (close(b, start) && close(a, end));
                }
                _ => return Err(INVALID),
            }
        }
        match self.limits {
            Limits::Span { .. } if covers_axis => Ok(()),
            Limits::FixedBody {
                socket_separation,
                lead_floors,
                start_share,
            } if socket_separation > 0.0
                && lead_floors.iter().all(|&floor| floor > 0.0)
                && (0.0..=1.0).contains(&start_share)
                && count[0] > 0
                && matches!(count[1..], [1, 1, 0..=1, 0] | [0, 0, 1, 0]) =>
            {
                Ok(())
            }
            _ => Err(INVALID),
        }
    }
}

fn object_map(objects: &[PageObject]) -> HashMap<&str, &PageObject> {
    objects
        .iter()
        .map(|object| (object.fields().id.as_str(), object))
        .collect()
}

/// Drop invalidated existing rules on normal edits. Newly supplied rules still face strict validation.
pub(crate) fn reconcile(before: &[PageObject], after: &mut [PageObject]) {
    let old = object_map(before);
    let next = object_map(after);
    let clear: HashSet<_> = after
        .iter()
        .filter_map(|group| {
            let PageObject::Group {
                fields,
                child_ids,
                stretch: Some(rule),
            } = group
            else {
                return None;
            };
            let Some(PageObject::Group {
                fields: previous,
                stretch: Some(prior),
                ..
            }) = old.get(fields.id.as_str())
            else {
                return None;
            };
            if prior != rule {
                return None;
            }
            let valid = rule.validate(fields, child_ids, &next).is_ok();
            // A stretch commit changes the local port span. With unchanged ports, a body edit
            // must preserve each rigid part in the group frame.
            let body_edited = fields.ports == previous.ports
                && rule.parts.iter().any(|part| {
                    if part.role != Role::Body {
                        return false;
                    }
                    let (
                        Some(PageObject::Shape {
                            fields: a,
                            geometry: ga,
                            ..
                        }),
                        Some(PageObject::Shape {
                            fields: b,
                            geometry: gb,
                            ..
                        }),
                    ) = (
                        old.get(part.child_id.as_str()),
                        next.get(part.child_id.as_str()),
                    )
                    else {
                        return true;
                    };
                    ga != gb
                        || !close(
                            local(previous, point(a.x, a.y)),
                            local(fields, point(b.x, b.y)),
                        )
                        || (a.scale / previous.scale - b.scale / fields.scale).abs() > EPS
                        || (a.rotation - previous.rotation - b.rotation + fields.rotation)
                            .to_radians()
                            .sin()
                            .abs()
                            > EPS
                        || (a.rotation - previous.rotation - b.rotation + fields.rotation)
                            .to_radians()
                            .cos()
                            < 0.0
                });
            (!valid || body_edited).then(|| fields.id.clone())
        })
        .collect();
    for object in after {
        if let PageObject::Group {
            fields, stretch, ..
        } = object
            && clear.contains(&fields.id)
        {
            *stretch = None;
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StretchRequest {
    pub page_id: String,
    pub revision: u64,
    pub group_id: String,
    pub port: usize,
    pub endpoint: Point,
    pub modified_at: String,
}

/// Solve in the original group frame. The opposite endpoint and group scale stay fixed.
fn frame(
    fields: &ObjectFields,
    a: Point,
    b: Point,
    start_moved: bool,
    endpoint: Point,
    minimum: f64,
) -> (ObjectFields, f64) {
    let fixed = to_page_point(fields, if start_moved { b } else { a });
    let delta = if start_moved {
        sub(fixed, endpoint)
    } else {
        sub(endpoint, fixed)
    };
    let distance = delta.x.hypot(delta.y);
    let length = (distance / fields.scale).max(minimum);
    let rotation = if distance == 0.0 {
        fields.rotation
    } else {
        delta.y.atan2(delta.x).to_degrees()
    };
    let anchor = if start_moved {
        point(a.x + length, a.y)
    } else {
        a
    };
    let origin = sub(fixed, rotate(mul(anchor, fields.scale), rotation));
    let mut next = fields.clone();
    (next.x, next.y, next.rotation) = (origin.x, origin.y, rotation);
    (next, length)
}

pub fn apply(objects: &mut [PageObject], request: &StretchRequest) -> Result<bool, &'static str> {
    let index = objects
        .iter()
        .position(|object| object.fields().id == request.group_id)
        .ok_or(INVALID)?;
    let PageObject::Group {
        fields,
        child_ids,
        stretch: Some(rule),
    } = &objects[index]
    else {
        return Err(INVALID);
    };
    rule.validate(fields, child_ids, &object_map(objects))?;
    let ports = fields.ports.as_ref().ok_or(INVALID)?;
    let [a, b] = ordered_ports(ports, rule.axis)?;
    if !rule.axis.contains(&request.port) || !bounded(request.endpoint) {
        return Err(INVALID);
    }
    let original = fields.clone();
    let rule = rule.clone();
    let child_ids = child_ids.clone();
    let (mut next, length) = frame(
        &original,
        a,
        b,
        request.port == rule.axis[0],
        request.endpoint,
        rule.limits.minimum(),
    );
    let end = point(a.x + length, a.y);
    if close(to_page_point(&next, a), to_page_point(&original, a))
        && close(to_page_point(&next, end), to_page_point(&original, b))
    {
        return Ok(false);
    }
    next.ports.as_mut().ok_or(INVALID)?[rule.axis[1]] = end;
    next.modified_at.clone_from(&request.modified_at);
    let old_length = b.x - a.x;
    let ratio = length / old_length;
    let mut updates = Vec::with_capacity(rule.parts.len());
    for part in &rule.parts {
        let child_index = objects
            .iter()
            .position(|object| object.fields().id == part.child_id)
            .ok_or(INVALID)?;
        let mut child = objects[child_index].clone();
        let PageObject::Shape {
            fields, geometry, ..
        } = &mut child
        else {
            return Err(INVALID);
        };
        let before = fields.clone();
        let mut origin = local(&original, point(fields.x, fields.y));
        let relative = fields.rotation - original.rotation;
        match part.role {
            Role::Body => {
                origin.x += rule.limits.sockets(length)[0] - rule.limits.sockets(old_length)[0];
            }
            Role::Span => {
                origin.x = a.x + (origin.x - a.x) * ratio;
                let stretch = |p| {
                    let v = rotate(p, relative);
                    rotate(point(v.x * ratio, v.y), -relative)
                };
                match geometry {
                    ShapeGeometry::Line { start, end } => {
                        *start = stretch(*start);
                        *end = stretch(*end);
                    }
                    ShapeGeometry::Spline { nodes, .. } => {
                        for node in nodes {
                            node.point = stretch(node.point);
                            node.handle_in = node.handle_in.map(stretch);
                            node.handle_out = node.handle_out.map(stretch);
                        }
                    }
                    ShapeGeometry::Rectangle { width_pt, .. } => *width_pt *= ratio,
                    ShapeGeometry::Ellipse { .. } => return Err(INVALID),
                }
            }
            _ => {}
        }
        let position = to_page_point(&next, origin);
        (fields.x, fields.y, fields.rotation) = (position.x, position.y, next.rotation + relative);
        fields.modified_at.clone_from(&request.modified_at);
        if matches!(
            part.role,
            Role::StartLead | Role::EndLead | Role::ThroughLine
        ) {
            let ShapeGeometry::Line {
                start,
                end: line_end,
            } = geometry
            else {
                return Err(INVALID);
            };
            let side = usize::from(part.role == Role::EndLead);
            let (port, inner) = if part.role == Role::ThroughLine {
                (a, end)
            } else {
                (
                    [a, end][side],
                    point(a.x + rule.limits.sockets(length)[side], a.y),
                )
            };
            let start_was_port = close(
                local(&original, to_page_point(&before, *start)),
                [a, b][side],
            );
            let [p, q] = [port, inner].map(|p| local(fields, to_page_point(&next, p)));
            (*start, *line_end) = if start_was_port { (p, q) } else { (q, p) };
        }
        updates.push((child_index, child));
    }
    rule.validate(
        &next,
        &child_ids,
        &updates
            .iter()
            .map(|(_, child)| (child.fields().id.as_str(), child))
            .collect(),
    )?;
    *objects[index].fields_mut() = next;
    for (index, child) in updates {
        objects[index] = child;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FrameCase {
        name: String,
        group: TestFrame,
        start: Point,
        end: Point,
        start_moved: bool,
        endpoint: Point,
        minimum: f64,
        expected: [Point; 2],
    }

    #[derive(Deserialize)]
    struct TestFrame {
        x: f64,
        y: f64,
        rotation: f64,
        scale: f64,
    }

    #[test]
    fn endpoint_frames_match_shared_fixtures() {
        let cases: Vec<FrameCase> =
            serde_json::from_str(include_str!("../../../fixtures/stretch-endpoints.json")).unwrap();
        for case in cases {
            let fields = ObjectFields {
                id: "group".into(),
                x: case.group.x,
                y: case.group.y,
                rotation: case.group.rotation,
                scale: case.group.scale,
                group_id: None,
                z_index: 0,
                reading_order: 0,
                created_at: String::new(),
                modified_at: String::new(),
                ports: None,
                addon_data: None,
            };
            let (next, length) = frame(
                &fields,
                case.start,
                case.end,
                case.start_moved,
                case.endpoint,
                case.minimum,
            );
            let end = point(case.start.x + length, case.start.y);
            assert!(
                close(to_page_point(&next, case.start), case.expected[0]),
                "{}",
                case.name
            );
            assert!(
                close(to_page_point(&next, end), case.expected[1]),
                "{}",
                case.name
            );
            assert!((next.scale - fields.scale).abs() < EPS);
        }
    }

    #[derive(Deserialize)]
    struct GeometryCase {
        name: String,
        length: f64,
        requested: f64,
        limits: Limits,
        parts: Vec<GeometryPart>,
        checks: Vec<GeometryCheck>,
    }

    #[derive(Deserialize)]
    struct GeometryPart {
        role: Role,
        x: f64,
        y: f64,
        geometry: ShapeGeometry,
    }

    #[derive(Deserialize)]
    struct GeometryCheck {
        index: usize,
        x: f64,
        y: f64,
        geometry: ShapeGeometry,
    }

    fn same_geometry(a: &ShapeGeometry, b: &ShapeGeometry) -> bool {
        use serde_json::Value;
        fn same(a: &Value, b: &Value) -> bool {
            match (a, b) {
                (Value::Object(a), Value::Object(b)) => {
                    a.len() == b.len()
                        && a.iter()
                            .all(|(key, a)| b.get(key).is_some_and(|b| same(a, b)))
                }
                (Value::Array(a), Value::Array(b)) => {
                    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
                }
                (Value::Number(a), Value::Number(b)) => {
                    (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < EPS
                }
                _ => a == b,
            }
        }
        same(
            &serde_json::to_value(a).unwrap(),
            &serde_json::to_value(b).unwrap(),
        )
    }

    #[test]
    fn geometry_matches_shared_cases_and_recovers_after_inverse_stretch() {
        let cases: Vec<GeometryCase> =
            serde_json::from_str(include_str!("../../../fixtures/stretch-geometry.json")).unwrap();
        for case in cases {
            let fields = |id: String, x, y, group_id| ObjectFields {
                id,
                x,
                y,
                group_id,
                scale: 1.0,
                rotation: 0.0,
                z_index: 0,
                reading_order: 0,
                created_at: String::new(),
                modified_at: String::new(),
                ports: None,
                addon_data: None,
            };
            let style = serde_json::from_value::<crate::ShapeStyle>(serde_json::json!({
                "strokeColor": "#000000", "strokeWidthPt": 2, "fillColor": null, "opacity": 1,
            }))
            .unwrap();
            let parts: Vec<Part> = case
                .parts
                .iter()
                .enumerate()
                .map(|(index, part)| Part {
                    child_id: index.to_string(),
                    role: part.role,
                })
                .collect();
            let children: Vec<PageObject> = case
                .parts
                .iter()
                .zip(&parts)
                .map(|(authored, part)| PageObject::Shape {
                    fields: fields(
                        part.child_id.clone(),
                        authored.x,
                        authored.y,
                        Some("group".into()),
                    ),
                    geometry: authored.geometry.clone(),
                    style: style.clone(),
                })
                .collect();
            let mut objects = children;
            let mut group = fields("group".into(), 0.0, 0.0, None);
            group.ports = Some(vec![point(0.0, 0.0), point(case.length, 0.0)]);
            objects.push(PageObject::Group {
                fields: group,
                child_ids: parts.iter().map(|part| part.child_id.clone()).collect(),
                stretch: Some(StretchRule {
                    axis: [0, 1],
                    parts,
                    limits: case.limits,
                }),
            });
            let mut request = StretchRequest {
                page_id: "fixture".into(),
                revision: 0,
                group_id: "group".into(),
                port: 1,
                endpoint: point(case.requested, 0.0),
                modified_at: String::new(),
            };
            assert!(
                apply(&mut objects, &request).unwrap_or_else(|e| panic!("{}: {e}", case.name)),
                "{}",
                case.name
            );
            for expected in case.checks {
                let PageObject::Shape {
                    fields,
                    geometry,
                    style: actual_style,
                } = &objects[expected.index]
                else {
                    unreachable!()
                };
                assert!(
                    close(point(fields.x, fields.y), point(expected.x, expected.y)),
                    "{}",
                    case.name
                );
                assert!(
                    same_geometry(geometry, &expected.geometry),
                    "{}: {geometry:?}",
                    case.name
                );
                assert_eq!(actual_style, &style);
            }
            request.endpoint.x = case.length;
            assert!(apply(&mut objects, &request).unwrap(), "{}", case.name);
            for (actual, authored) in objects.iter().zip(case.parts) {
                let PageObject::Shape {
                    fields, geometry, ..
                } = actual
                else {
                    unreachable!()
                };
                assert!(
                    close(point(fields.x, fields.y), point(authored.x, authored.y)),
                    "{}",
                    case.name
                );
                assert!(same_geometry(geometry, &authored.geometry), "{}", case.name);
            }
        }
    }
}
