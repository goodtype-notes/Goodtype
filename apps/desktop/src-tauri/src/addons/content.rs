use super::*;
use goodtype_core::{ShapeCap, ShapeGeometry, ShapeStyle, StrokePattern, template::PageTemplate};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;

pub(super) const MAX_PACK_BYTES: u64 = 16 * MIB;
const MAX_CONTENT_JSON: usize = MIB as usize;

fn valid_dimension(value: f64) -> bool {
    value > 0.0 && value <= goodtype_core::MAX_PAGE_DIMENSION_PT
}

#[derive(Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
#[derive(Default)]
pub(super) struct Contents {
    shapes: Vec<String>,
    templates: Vec<String>,
    palettes: Vec<String>,
    themes: Vec<String>,
    typst_packages: Vec<String>,
}

impl Contents {
    pub(super) fn valid(&self) -> bool {
        let lists = [
            ("shapes", &self.shapes),
            ("templates", &self.templates),
            ("palettes", &self.palettes),
            ("themes", &self.themes),
            ("typst", &self.typst_packages),
        ];
        let mut seen = HashSet::new();
        lists.iter().any(|(_, paths)| !paths.is_empty())
            && self.typst_packages.len() <= 16
            && lists.into_iter().all(|(kind, files)| {
                files.len() <= 32
                    && files.iter().all(|file| {
                        seen.insert(file.to_ascii_lowercase())
                            && !file.ends_with('/')
                            && safe_components(file).is_ok_and(|parts| {
                                parts[0] == "content"
                                    && parts.get(1) == Some(&kind)
                                    && if kind == "typst" {
                                        parts.len() == 5
                                            && parts[2] == "preview"
                                            && typst_syntax::is_ident(parts[3])
                                            && manifest::release_version(parts[4])
                                    } else {
                                        parts.len() == 3 && file.ends_with(".json")
                                    }
                            })
                    })
            })
    }
}

fn decode<T: DeserializeOwned>(root: &Path, name: &str) -> Result<T, String> {
    let bytes = read_bounded(&root.join(name), MAX_CONTENT_JSON)?;
    let session::Strict(value) = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !value.is_object() {
        return Err("content JSON must be an object".into());
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

// Core notebook readers tolerate additive fields. Content packs use the same types with
// a stricter boundary, using their canonical serialization to identify recognized fields.
fn strict_core<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned + Serialize,
{
    use serde_json::Value;
    fn known_fields(input: &Value, canonical: &Value) -> bool {
        match (input, canonical) {
            (Value::Object(input), Value::Object(canonical)) => input.iter().all(|(key, value)| {
                canonical
                    .get(key)
                    .is_some_and(|field| known_fields(value, field))
            }),
            (Value::Array(input), Value::Array(canonical)) => {
                input.len() == canonical.len()
                    && input
                        .iter()
                        .zip(canonical)
                        .all(|(value, field)| known_fields(value, field))
            }
            (Value::Object(_) | Value::Array(_), _) | (_, Value::Object(_) | Value::Array(_)) => {
                false
            }
            _ => true,
        }
    }
    let input = Value::deserialize(deserializer)?;
    let value: T = serde_json::from_value(input.clone()).map_err(serde::de::Error::custom)?;
    let canonical = serde_json::to_value(&value).map_err(serde::de::Error::custom)?;
    if !known_fields(&input, &canonical) {
        return Err(serde::de::Error::custom(
            "unknown field or non-object core value",
        ));
    }
    Ok(value)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Library {
    id: String,
    label: String,
    items: Vec<Item>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Item {
    id: String,
    label: String,
    search: String,
    width_pt: f64,
    height_pt: f64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "strict_core"
    )]
    ports: Option<Vec<goodtype_core::ShapePoint>>,
    objects: Vec<ContentObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stretch: Option<ItemStretch>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ItemStretch {
    axis: [usize; 2],
    roles: Vec<goodtype_core::stretch::Role>,
    mode: StretchMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lead_floors: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    virtual_sockets: Option<[f64; 2]>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum StretchMode {
    FixedBody,
    Span,
}

#[derive(Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ContentObject {
    Shape {
        x: f64,
        y: f64,
        #[serde(deserialize_with = "strict_core")]
        geometry: ShapeGeometry,
        style: Style,
    },
    Typst {
        x: f64,
        y: f64,
        source: String,
        layout_width_pt: f64,
        scale: f64,
    },
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Style {
    #[serde(skip_serializing_if = "ShapeCap::is_none")]
    start_cap: ShapeCap,
    #[serde(skip_serializing_if = "ShapeCap::is_none")]
    end_cap: ShapeCap,
    #[serde(deserialize_with = "present")]
    stroke_color: Option<String>,
    #[serde(deserialize_with = "present")]
    stroke_width_pt: Option<f64>,
    fill_color: Option<String>,
    #[serde(deserialize_with = "present")]
    opacity: Option<f64>,
    #[serde(deserialize_with = "present")]
    pattern: Option<StrokePattern>,
    #[serde(deserialize_with = "present")]
    pattern_spacing: Option<f64>,
    #[serde(deserialize_with = "present")]
    pattern_offset_pt: Option<f64>,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl Style {
    fn resolve(&self, color: &str, width: f64) -> ShapeStyle {
        ShapeStyle {
            start_cap: self.start_cap,
            end_cap: self.end_cap,
            stroke_color: self.stroke_color.as_deref().unwrap_or(color).into(),
            stroke_width_pt: self.stroke_width_pt.unwrap_or(width),
            fill_color: self
                .fill_color
                .as_deref()
                .map(|fill| if fill == "pen" { color } else { fill }.into()),
            opacity: self.opacity.unwrap_or(1.0),
            pattern: self.pattern.unwrap_or(StrokePattern::Solid),
            pattern_spacing: self.pattern_spacing.unwrap_or(1.0),
            pattern_offset_pt: self.pattern_offset_pt.unwrap_or(0.0),
        }
    }
}

impl Item {
    fn stretch_rule(
        &self,
        child_ids: &[String],
        scale: f64,
    ) -> Result<Option<goodtype_core::stretch::StretchRule>, &'static str> {
        use goodtype_core::stretch::{Limits, Part, Role, StretchRule, inner_lead, ordered_ports};

        let Some(declaration) = &self.stretch else {
            return Ok(None);
        };
        let invalid = "invalid content stretch declaration";
        let [start, end] = ordered_ports(self.ports.as_deref().ok_or(invalid)?, declaration.axis)?;
        if declaration.roles.len() != self.objects.len()
            || child_ids.len() != self.objects.len()
            || ![start, end].iter().all(|p| {
                (0.0..=self.width_pt).contains(&p.x) && (0.0..=self.height_pt).contains(&p.y)
            })
        {
            return Err(invalid);
        }
        let length = end.x - start.x;
        let limits = match declaration.mode {
            StretchMode::Span
                if declaration.lead_floors.is_some() || declaration.virtual_sockets.is_some() =>
            {
                return Err(invalid);
            }
            StretchMode::Span => Limits::Span {
                minimum_length: (length / 2.0).max(4.0) * scale,
            },
            StretchMode::FixedBody => {
                let ItemStretch {
                    lead_floors,
                    virtual_sockets,
                    ..
                } = declaration;
                let lead = |role, port| {
                    let index = declaration
                        .roles
                        .iter()
                        .position(|candidate| *candidate == role)
                        .ok_or(invalid)?;
                    let ContentObject::Shape {
                        x,
                        y,
                        geometry: ShapeGeometry::Line { start, end },
                        ..
                    } = &self.objects[index]
                    else {
                        return Err(invalid);
                    };
                    inner_lead(
                        [*start, *end].map(|p| goodtype_core::ShapePoint {
                            x: x + p.x,
                            y: y + p.y,
                        }),
                        port,
                    )
                };
                let sockets = if let Some(sockets) = virtual_sockets {
                    if declaration
                        .roles
                        .iter()
                        .any(|role| matches!(role, Role::StartLead | Role::EndLead))
                    {
                        return Err(invalid);
                    }
                    *sockets
                } else {
                    [
                        lead(Role::StartLead, start)?.x - start.x,
                        lead(Role::EndLead, end)?.x - start.x,
                    ]
                };
                Limits::fixed(
                    length * scale,
                    sockets.map(|x| x * scale),
                    lead_floors.unwrap_or([4.0; 2]).map(|x| x * scale),
                )?
            }
        };
        Ok(Some(StretchRule {
            axis: declaration.axis,
            parts: child_ids
                .iter()
                .zip(&declaration.roles)
                .map(|(id, &role)| Part {
                    child_id: id.clone(),
                    role,
                })
                .collect(),
            limits,
        }))
    }

    fn validate_stretch(&self) -> Result<(), &'static str> {
        if self.stretch.is_none() {
            return Ok(());
        }
        let ids: Vec<_> = (0..self.objects.len())
            .map(|index| index.to_string())
            .collect();
        let Some(rule) = self.stretch_rule(&ids, 1.0)? else {
            return Ok(());
        };
        let fields = |id, x, y, group_id, ports| ObjectFields {
            id,
            x,
            y,
            rotation: 0.0,
            scale: 1.0,
            z_index: 0,
            reading_order: 0,
            group_id,
            ports,
            addon_data: None,
            created_at: String::new(),
            modified_at: String::new(),
        };
        let mut children = Vec::with_capacity(ids.len());
        for (object, id) in self.objects.iter().zip(&ids) {
            let ContentObject::Shape {
                x,
                y,
                geometry,
                style,
            } = object
            else {
                return Err("stretch parts must be shapes");
            };
            children.push(PageObject::Shape {
                fields: fields(id.clone(), *x, *y, Some("group".into()), None),
                geometry: geometry.clone(),
                style: style.resolve("#000000", 1.0),
            });
        }
        rule.validate(
            &fields("group".into(), 0.0, 0.0, None, self.ports.clone()),
            &ids,
            &children
                .iter()
                .map(|child| (child.fields().id.as_str(), child))
                .collect(),
        )
    }

    fn validate(&self) -> Result<(), &'static str> {
        if !valid_dimension(self.width_pt)
            || !valid_dimension(self.height_pt)
            || !(1..=64).contains(&self.objects.len())
        {
            return Err("invalid content item dimensions or object count");
        }
        if self.ports.as_ref().is_some_and(|ports| {
            ports.len() > goodtype_core::addon_data::MAX_PORTS
                || ports
                    .iter()
                    .any(|point| !point.x.is_finite() || !point.y.is_finite())
                || (self.objects.len() == 1
                    && matches!(self.objects[0], ContentObject::Typst { .. }))
        }) {
            return Err("invalid content item ports");
        }
        for object in &self.objects {
            let valid = match object {
                ContentObject::Shape {
                    x,
                    y,
                    geometry,
                    style,
                } => {
                    // Strokes may extend past the item edge; the outline itself may not.
                    super::batch::validate_shape(geometry, &style.resolve("#000000", 1.0))?;
                    super::batch::shape_within(
                        *x,
                        *y,
                        geometry,
                        (self.width_pt, self.height_pt, 0.0),
                    )
                }
                ContentObject::Typst {
                    x,
                    y,
                    source,
                    layout_width_pt,
                    scale,
                } => {
                    let overhang = self.width_pt.max(self.height_pt) / 2.0;
                    !source.is_empty()
                        && source.len() <= 2048
                        && (0.0 < *layout_width_pt
                            && *layout_width_pt <= goodtype_typst::MAX_BLOCK_WIDTH_PT)
                        && scale.is_finite()
                        && *scale > 0.0
                        && (-overhang..=self.width_pt + overhang).contains(x)
                        && x + layout_width_pt * scale <= self.width_pt + overhang
                        && (-overhang..=self.height_pt + overhang).contains(y)
                }
            };
            if !valid {
                return Err("content object exceeds the item bounds");
            }
        }
        self.validate_stretch()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Template {
    label: String,
    #[serde(deserialize_with = "strict_core")]
    template: PageTemplate,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Palette {
    id: String,
    label: String,
    colors: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Theme {
    id: String,
    label: String,
    tokens: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplatesFile {
    templates: Vec<Template>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PalettesFile {
    palettes: Vec<Palette>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemesFile {
    themes: Vec<Theme>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    id: &'static str,
    strings: manifest::Strings,
    shapes: Vec<Library>,
    templates: Vec<Template>,
    palettes: Vec<Palette>,
    themes: Vec<Theme>,
    typst_packages: Vec<String>,
    #[serde(skip)]
    package_scope: Vec<String>,
}

pub(super) fn load(
    addon: &Addon,
    root: &Path,
    strings: &manifest::Strings,
    contents: &Contents,
) -> Result<Pack, String> {
    let mut pack = Pack {
        id: addon.id,
        strings: manifest::Strings {
            en: strings.en.clone(),
            de: strings.de.clone(),
        },
        shapes: vec![],
        templates: vec![],
        palettes: vec![],
        themes: vec![],
        typst_packages: vec![],
        package_scope: vec![],
    };
    let mut library_ids = HashSet::new();
    let label = |key: &str| strings.en.contains_key(key);
    for file in &contents.shapes {
        let library: Library = decode(root, file)?;
        let mut item_ids = HashSet::new();
        if !manifest::identifier(&library.id)
            || !library_ids.insert(library.id.clone())
            || !label(&library.label)
            || library.items.is_empty()
        {
            return Err("invalid shape library".into());
        }
        for item in &library.items {
            if !manifest::identifier(&item.id)
                || !item_ids.insert(&item.id)
                || !label(&item.label)
                || !label(&item.search)
            {
                return Err("invalid shape identity or label".into());
            }
            item.validate()?;
        }
        pack.shapes.push(library);
        if pack
            .shapes
            .iter()
            .map(|library| library.items.len())
            .sum::<usize>()
            > 512
        {
            return Err("too many shapes in content pack".into());
        }
    }
    for file in &contents.templates {
        pack.templates
            .extend(decode::<TemplatesFile>(root, file)?.templates);
    }
    let mut ids = HashSet::new();
    let prefix = format!("{}.", addon.id);
    if pack.templates.len() > 32
        || pack.templates.iter().any(|item| {
            !label(&item.label)
                || !ids.insert(&item.template.id)
                || !item
                    .template
                    .id
                    .strip_prefix(&prefix)
                    .is_some_and(manifest::identifier)
                || goodtype_core::template::validate(&item.template).is_err()
        })
    {
        return Err("invalid content templates".into());
    }
    for file in &contents.palettes {
        pack.palettes
            .extend(decode::<PalettesFile>(root, file)?.palettes);
    }
    let mut ids = HashSet::new();
    if pack.palettes.len() > 16
        || pack.palettes.iter().any(|item| {
            !label(&item.label)
                || !manifest::identifier(&item.id)
                || !ids.insert(&item.id)
                || !(2..=24).contains(&item.colors.len())
                || item.colors.iter().any(|color| hex_rgb(color).is_none())
                || item
                    .colors
                    .iter()
                    .map(|color| color.to_ascii_lowercase())
                    .collect::<HashSet<_>>()
                    .len()
                    != item.colors.len()
        })
    {
        return Err("invalid content palettes".into());
    }
    for file in &contents.themes {
        pack.themes.extend(decode::<ThemesFile>(root, file)?.themes);
    }
    let mut ids = HashSet::new();
    if pack.themes.len() > 8
        || pack.themes.iter().any(|item| {
            !label(&item.label)
                || !manifest::identifier(&item.id)
                || !ids.insert(&item.id)
                || !valid_theme(&item.tokens)
        })
    {
        return Err("invalid content theme or insufficient contrast".into());
    }
    for relative in &contents.typst_packages {
        let parts = safe_components(relative)?;
        let bytes = read_bounded(&root.join(relative).join("typst.toml"), MAX_CONTENT_JSON)?;
        let manifest: typst_syntax::package::PackageManifest =
            toml::from_str(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if manifest.package.name != parts[3] || manifest.package.version.to_string() != parts[4] {
            return Err("Typst package identity does not match its path".into());
        }
        safe_components(&manifest.package.entrypoint)?;
        if !root
            .join(relative)
            .join(manifest.package.entrypoint.as_str())
            .is_file()
        {
            return Err("missing Typst package entrypoint".into());
        }
        pack.package_scope.push(parts[2..].join("/"));
        pack.typst_packages
            .push(format!("{} {}", parts[3], parts[4]));
    }
    Ok(pack)
}

fn hex_rgb(value: &str) -> Option<[f64; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some([16, 8, 0].map(|shift| f64::from((rgb >> shift) & 255) / 255.0))
}

fn color(value: &str) -> Option<([f64; 3], f64)> {
    if let Some(rgb) = hex_rgb(value) {
        return Some((rgb, 1.0));
    }
    let value = value.strip_prefix("rgb(")?.strip_suffix(')')?;
    let [r, g, b, "/", alpha]: [&str; 5] = value
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .try_into()
        .ok()?
    else {
        return None;
    };
    let r = r.parse::<u8>().ok()?;
    let g = g.parse::<u8>().ok()?;
    let b = b.parse::<u8>().ok()?;
    let alpha = alpha.strip_suffix('%')?;
    if !alpha
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    let alpha = alpha.parse::<f64>().ok()?;
    (0.0..=100.0)
        .contains(&alpha)
        .then_some(([r, g, b].map(|v| f64::from(v) / 255.0), alpha / 100.0))
}

fn over((rgb, alpha): ([f64; 3], f64), under: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|index| rgb[index] * alpha + under[index] * (1.0 - alpha))
}

fn luminance(rgb: [f64; 3]) -> f64 {
    let [r, g, b] = rgb.map(|v| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn valid_theme(tokens: &BTreeMap<String, String>) -> bool {
    const ALLOW: &[&str] = &[
        "--charcoal",
        "--surround",
        "--panel",
        "--text",
        "--muted",
        "--quiet",
        "--blueprint",
        "--blueprint-light",
        "--blueprint-hover",
        "--on-accent",
        "--accent-tint",
        "--accent-fill",
        "--accent-edge",
        "--accent-select",
        "--oxide",
        "--oxide-hover",
        "--oxide-text",
        "--oxide-wash",
        "--amber",
        "--amber-text",
        "--sage",
        "--sage-text",
        "--fill-soft",
        "--wash",
        "--wash-strong",
        "--edge-soft",
        "--edge",
        "--edge-strong",
        "--inset",
        "--track",
    ];
    if tokens.is_empty()
        || tokens
            .iter()
            .any(|(key, value)| !ALLOW.contains(&key.as_str()) || color(value).is_none())
    {
        return false;
    }
    let get =
        |key: &str, default: &str| color(tokens.get(key).map_or(default, String::as_str)).unwrap();
    let legible = |fg, backgrounds: [[f64; 3]; 2]| {
        let [over_black, over_white] = backgrounds.map(|bg| {
            let a = luminance(over(fg, bg));
            let b = luminance(bg);
            ((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5).then_some(a > b)
        });
        over_black.is_some() && over_black == over_white
    };
    // An alpha surface can cover light paper as well as dark chrome. Test both
    // extremes and reject foregrounds whose contrast reverses between them.
    [("--panel", "#23272f"), ("--surround", "#1b1e24")]
        .into_iter()
        .all(|(surface, default)| {
            let background = get(surface, default);
            let backgrounds = [over(background, [0.0; 3]), over(background, [1.0; 3])];
            [
                ("--text", "#e9ebee"),
                ("--muted", "#aeb5be"),
                ("--quiet", "#8a929c"),
            ]
            .into_iter()
            .all(|(name, default)| legible(get(name, default), backgrounds))
                && legible(
                    get("--on-accent", "#0e1b31"),
                    backgrounds.map(|bg| over(get("--blueprint", "#4c8df0"), bg)),
                )
        })
}

fn installed_pack(
    app: &tauri::AppHandle,
    addon: &'static Addon,
    include_disabled: bool,
) -> Result<Pack, String> {
    if !addon.is_content()
        || (!include_disabled
            && !manager::policy(app)?
                .addons
                .get(addon.id)
                .is_some_and(|p| p.enabled))
    {
        return Err("unavailable".into());
    }
    let (version, path) = install_dir(app, addon)?.active().ok_or("unavailable")?;
    let declaration =
        manager::installed(addon, &path, &version, false).map_err(manager::manifest_problem)?;
    let contents = declaration.contents.ok_or("invalid-manifest")?;
    load(addon, &path, &declaration.strings, &contents)
}

#[tauri::command]
pub async fn addon_content(
    app: tauri::AppHandle,
    id: Option<String>,
    include_disabled: bool,
) -> Result<Vec<Pack>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        CATALOG
            .iter()
            .filter(|addon| id.as_deref().is_none_or(|id| addon.id == id))
            .filter_map(|addon| installed_pack(&app, addon, include_disabled).ok())
            .collect()
    })
    .await
    .map_err(|_| "installation-read-failed".into())
}
use goodtype_core::{ObjectFields, PageObject, storage};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeInsertion {
    addon_id: String,
    library_id: String,
    item_id: String,
    revision: u64,
    point: Option<goodtype_core::ShapePoint>,
    scale: Option<f64>,
    pen_color: String,
    pen_width_pt: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertedShape {
    #[serde(flatten)]
    result: storage::HistoryResult,
    selected_ids: Vec<String>,
}

pub(crate) fn insert(
    app: &tauri::AppHandle,
    root: &Path,
    page_id: &str,
    history: &mut storage::NotebookHistory,
    request: ShapeInsertion,
) -> Result<InsertedShape, storage::StorageError> {
    let invalid = |message: &str| storage::StorageError::InvalidNotebook(message.into());
    let importers = app.state::<Importers>();
    let (job, addon) = importers
        .get(&request.addon_id)
        .map_err(|_| invalid("addon-unavailable"))?;
    let _guard = job
        .lock_idle()
        .ok_or_else(|| invalid("addon-unavailable"))?;
    let pack = installed_pack(app, addon, false).map_err(|_| invalid("addon-unavailable"))?;
    let item = pack
        .shapes
        .iter()
        .find(|library| library.id == request.library_id)
        .and_then(|library| library.items.iter().find(|item| item.id == request.item_id))
        .ok_or_else(|| invalid("addon-unavailable"))?;
    commit_shape(
        root,
        page_id,
        history,
        &request,
        item,
        |source, width_pt| {
            goodtype_typst::with_content_package_scope(pack.package_scope.clone(), || {
                goodtype_typst::compile_block_granted(
                    root,
                    &goodtype_typst::CompileRequest {
                        source: source.into(),
                        width_pt,
                        generation: 0,
                        allow_remote_packages: app
                            .state::<crate::settings::RemotePackages>()
                            .allowed(),
                    },
                    &goodtype_typst::GrantedFiles::new(),
                )
            })
            .ok()
            .and_then(|result| result.width_pt.zip(result.height_pt))
        },
    )
}

fn commit_shape(
    root: &Path,
    page_id: &str,
    history: &mut storage::NotebookHistory,
    request: &ShapeInsertion,
    item: &Item,
    compile_label: impl Fn(&str, f64) -> Option<(f64, f64)>,
) -> Result<InsertedShape, storage::StorageError> {
    let invalid = |message: &str| storage::StorageError::InvalidNotebook(message.into());
    let mut page = storage::open_page(root, page_id)?;
    if page.page.calculation.is_some() {
        return Err(invalid("content shapes require a notebook page"));
    }
    if page.page.revision != request.revision {
        return Err(storage::StorageError::RevisionConflict {
            expected: request.revision,
            found: page.page.revision,
        });
    }
    let scale = request.scale.unwrap_or(1.0);
    if !(0.25..=8.0).contains(&scale) {
        return Err(invalid("invalid content shape scale"));
    }
    let point = request.point.unwrap_or(goodtype_core::ShapePoint {
        x: page.page.geometry.width_pt / 2.0,
        y: page.page.geometry.height_pt / 2.0,
    });
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(invalid("invalid content shape position"));
    }
    let left = point.x - item.width_pt * scale / 2.0;
    let top = point.y - item.height_pt * scale / 2.0;
    let mut z = page
        .page
        .objects
        .iter()
        .map(|o| o.fields().z_index)
        .chain(
            page.ink_layers
                .iter()
                .flat_map(|layer| layer.strokes.iter().map(|s| s.z_index)),
        )
        .max()
        .unwrap_or(0);
    let group_id = (item.objects.len() > 1 || item.stretch.is_some())
        .then(|| format!("pack-{}", unique_suffix()));
    let stamp = crate::helper::now().to_string();
    let mut next_fields = |id, x, y, object_scale, group_id| -> Result<_, storage::StorageError> {
        z = z
            .checked_add(1)
            .ok_or_else(|| invalid("content shape layer overflow"))?;
        Ok(ObjectFields {
            id,
            x,
            y,
            rotation: 0.0,
            scale: object_scale,
            z_index: z,
            reading_order: 0,
            group_id,
            created_at: stamp.clone(),
            modified_at: stamp.clone(),
            addon_data: None,
            ports: None,
        })
    };
    let mut child_ids = Vec::new();
    for object in &item.objects {
        let id = format!("pack-{}", unique_suffix());
        let mut object = match object {
            ContentObject::Shape {
                x,
                y,
                geometry,
                style,
            } => super::batch::shape_object(
                next_fields(
                    id.clone(),
                    left + x * scale,
                    top + y * scale,
                    scale,
                    group_id.clone(),
                )?,
                geometry.clone(),
                style.resolve(&request.pen_color, request.pen_width_pt),
            )
            .map_err(invalid)?,
            ContentObject::Typst {
                x,
                y,
                source,
                layout_width_pt,
                scale: label_scale,
            } => {
                let (width, height) = compile_label(source, *layout_width_pt)
                    .ok_or_else(|| invalid("addon-compile-failed"))?;
                let overhang = item.width_pt.max(item.height_pt) / 2.0;
                if x + width * label_scale > item.width_pt + overhang
                    || y + height * label_scale > item.height_pt + overhang
                {
                    return Err(invalid("content shape label exceeds bounds"));
                }
                let source_path = format!("blocks/{id}.typ");
                page.blocks.push(storage::StoredFile {
                    path: source_path.clone(),
                    bytes: source.as_bytes().to_vec(),
                });
                PageObject::Typst {
                    fields: next_fields(
                        id.clone(),
                        left + x * scale,
                        top + y * scale,
                        scale * label_scale,
                        group_id.clone(),
                    )?,
                    source_path,
                    layout_width_pt: *layout_width_pt,
                    measured_width_pt: width,
                    measured_height_pt: height,
                }
            }
        };
        if group_id.is_none() {
            if let Some(ports) = &item.ports {
                // Pack points are relative to the item, whereas a leaf has its own origin.
                if let ContentObject::Shape { x, y, .. } = &item.objects[0] {
                    object.fields_mut().ports = Some(
                        ports
                            .iter()
                            .map(|point| goodtype_core::ShapePoint {
                                x: point.x - x,
                                y: point.y - y,
                            })
                            .collect(),
                    );
                }
            }
            object.fields_mut().reading_order = page
                .page
                .reading_order
                .len()
                .try_into()
                .map_err(|_| invalid("content shape reading order overflow"))?;
            page.page.reading_order.push(id.clone());
        }
        page.page.objects.push(object);
        child_ids.push(id);
    }
    if let Some(id) = group_id {
        let mut fields = next_fields(id.clone(), left, top, 1.0, None)?;
        fields.ports = item.ports.as_ref().map(|ports| {
            ports
                .iter()
                .map(|point| goodtype_core::ShapePoint {
                    x: point.x * scale,
                    y: point.y * scale,
                })
                .collect()
        });
        fields.reading_order = page
            .page
            .reading_order
            .len()
            .try_into()
            .map_err(|_| invalid("content shape reading order overflow"))?;
        page.page.reading_order.push(id);
        page.page.objects.push(PageObject::Group {
            fields,
            child_ids: child_ids.clone(),
            stretch: item.stretch_rule(&child_ids, scale).map_err(invalid)?,
        });
    }
    storage::raise_schema_version(&mut page);
    let result = storage::commit_notebook_with_undo(root, history, page)?;
    Ok(InsertedShape {
        result,
        selected_ids: child_ids,
    })
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn addon_sideload_content(
    app: tauri::AppHandle,
    ui_language: Option<String>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(folder) = app
        .dialog()
        .file()
        .set_title(crate::settings::dialog_label(
            ui_language.as_deref(),
            "Load local pack",
            "Lokales Paket laden",
        ))
        .blocking_pick_folder()
    else {
        return Ok(false);
    };
    let folder = folder.into_path().map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || sideload(&app, &folder))
        .await
        .map_err(|_| "invalid-manifest")?
        .inspect_err(|error| eprintln!("local addon load failed: {error}"))?;
    Ok(true)
}

#[cfg(any(debug_assertions, test))]
fn sideload_archive(folder: &Path) -> Result<(&'static Addon, String, tempfile::TempPath), String> {
    if !folder.is_absolute() {
        return Err("invalid content folder".into());
    }
    for ancestor in folder.ancestors() {
        if is_package_link(&fs::symlink_metadata(ancestor).map_err(|err| err.to_string())?) {
            return Err("content folder contains a link".into());
        }
    }
    let manifest_file = folder.join(manifest::FILE);
    let metadata = fs::symlink_metadata(&manifest_file)
        .map_err(|err| format!("{}: {err}", manifest_file.display()))?;
    if is_package_link(&metadata) || !metadata.is_file() {
        return Err("invalid content manifest file".into());
    }
    let bytes = read_bounded(&manifest_file, 64 * 1024)?;
    let session::Strict(declaration) =
        serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
    let id = declaration["id"].as_str().ok_or("invalid-manifest")?;
    let version = declaration["version"]
        .as_str()
        .filter(|s| manifest::release_version(s))
        .ok_or("invalid-manifest")?;
    let addon = catalog::CATALOG
        .iter()
        .copied()
        .find(|addon| addon.id == id)
        .ok_or("invalid-manifest")?;
    let declaration = manifest::parse(&bytes, addon, version).map_err(manager::manifest_problem)?;
    if !addon.is_content() && !declaration.uses_session() {
        return Err("invalid-manifest".into());
    }
    let mut files = vec![manifest::FILE.to_string()];
    let mut sources = std::collections::HashMap::new();
    if addon.is_content() {
        files.extend(
            package_files(&folder.join("content"))?
                .into_iter()
                .map(|path| format!("content/{path}")),
        );
    } else if !declaration.target_is_wasm() {
        let executable = addon.executable();
        // A package folder holds the executable beside the manifest; a source checkout holds it
        // in its release build output.
        let path = [
            folder.join(&executable),
            folder.join("target/release").join(&executable),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "{}: not found (also looked in target\release)",
                folder.join(&executable).display()
            )
        })?;
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() || is_package_link(&metadata) {
            return Err("invalid local addon executable".into());
        }
        sources.insert(executable.clone(), path);
        files.push(executable);
    }
    if let Some(wasm) = &declaration.wasm {
        let path = folder.join(&wasm.module);
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || is_package_link(&metadata) {
            return Err("invalid local module".into());
        }
        super::wasm::module_bytes(folder, &declaration)?;
        files.push(wasm.module.clone());
    }
    if files.len() > MAX_ARCHIVE_ENTRIES {
        return Err("content folder has too many files".into());
    }
    let temp = tempfile::NamedTempFile::new().map_err(|err| err.to_string())?;
    let mut zip = zip::ZipWriter::new(temp.reopen().map_err(|err| err.to_string())?);
    let top = package_folder(addon, version);
    let mut budget = if addon.is_content() {
        MAX_PACK_BYTES
    } else {
        MAX_UNPACKED_BYTES
    } as usize;
    for relative in files {
        let source = sources
            .get(&relative)
            .cloned()
            .unwrap_or_else(|| folder.join(&relative));
        let bytes = read_bounded(&source, budget)?;
        budget -= bytes.len();
        zip.start_file(
            format!("{top}/{relative}"),
            zip::write::SimpleFileOptions::default(),
        )
        .map_err(|err| err.to_string())?;
        zip.write_all(&bytes).map_err(|err| err.to_string())?;
    }
    zip.finish().map_err(|err| err.to_string())?;
    // Close every handle: installation opens the archive without write sharing.
    Ok((addon, version.into(), temp.into_temp_path()))
}

#[cfg(debug_assertions)]
fn sideload(app: &tauri::AppHandle, folder: &Path) -> Result<(), String> {
    let (addon, version, temp) = sideload_archive(folder)?;
    let importers = app.state::<Importers>();
    let (job, _) = importers.get(addon.id)?;
    let size = fs::metadata(&temp).map_err(|err| err.to_string())?.len();
    let hash = sha256_file(&temp)?;
    let release = PinnedRelease {
        wasm: None,
        version: &version,
        url: "",
        size,
        sha256: &hash,
        decoder_commit: "",
    };
    let dir = install_dir(app, addon)?;
    job.begin(Activity::Installing).map_err(|_| "busy")?;
    let result = if addon.is_content() {
        install_with_policy(
            &dir,
            &temp,
            &release,
            &|_| unreachable!("content packs never probe"),
            |_| Ok(()),
            |manifest| manager::activate(app, &dir, (false, None), &version, manifest, true),
        )
    } else {
        manager::install(
            app,
            &dir,
            &temp,
            &release,
            None,
            &mut InstallProblem::Policy,
        )
    };
    let mut state = job.state.lock().unwrap();
    state.activity = Some(Activity::Idle);
    if result.is_ok() {
        state.startup_invalid = false;
        state.incompatible_session = None;
    }
    result.map(|_| ())
}

pub(crate) fn setup(app: &tauri::AppHandle) -> Result<(), String> {
    goodtype_typst::set_content_package_directory(package_directory(app)?);
    Ok(())
}

fn package_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    crate::settings::config_file(
        app,
        if cfg!(debug_assertions) {
            "addon-typst-packages-dev"
        } else {
            "addon-typst-packages"
        },
    )
}

/// Retain verified offline packages for saved blocks that refer to them, including after
/// addon removal. Package versions are immutable and the user's Typst cache stays separate.
pub(super) fn retain_packages(
    app: &tauri::AppHandle,
    root: &Path,
    contents: &Contents,
) -> Result<(), String> {
    retain_package_files(&package_directory(app)?, root, contents)
}

fn retain_package_files(packages: &Path, root: &Path, contents: &Contents) -> Result<(), String> {
    for relative in &contents.typst_packages {
        let suffix = relative
            .strip_prefix("content/typst/")
            .ok_or("invalid Typst package path")?;
        let target = packages.join(suffix);
        if target.exists() {
            continue;
        }
        let parent = target.parent().ok_or("invalid Typst package path")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = tempfile::tempdir_in(packages).map_err(|e| e.to_string())?;
        let source = root.join(relative);
        let candidate = temp.path().join("package");
        fs::create_dir(&candidate).map_err(|e| e.to_string())?;
        for file in package_files(&source)? {
            let dest = candidate.join(&file);
            fs::create_dir_all(dest.parent().ok_or("invalid Typst package file")?)
                .map_err(|e| e.to_string())?;
            fs::copy(source.join(file), dest).map_err(|e| e.to_string())?;
        }
        if let Err(error) = fs::rename(candidate, &target)
            && !target.is_dir()
        {
            return Err(error.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
