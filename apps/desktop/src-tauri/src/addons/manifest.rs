use super::{
    contributions::{Field, Format, valid_fields},
    *,
};
use serde::de::{MapAccess, Visitor};
use std::collections::BTreeMap;

pub(super) const FILE: &str = "goodtype-addon.json";
pub(super) const API: &str = "goodtype-api 0.1";
const MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    PickedImportFile,
    DerivedImportArtwork,
    PublishImportBatch,
    SelectedInk,
    AddPageObjects,
    ReadPages,
    /// Create and change objects that carry this addon's data (tools, own-object commands).
    OwnObjects,
    /// Receive `page/changed` after a commit that touched an object its own objects depend on.
    ReactToChanges,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Transport {
    LegacyImportCli,
    StdioJsonrpc,
    Content,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Surface {
    LibraryImport,
    InkSelection,
    InsertForm,
    PageGenerator,
    Export,
    /// Shown in the selection menu when the selection holds objects with this addon's data.
    OwnObject,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ToolKind {
    Place,
    Connect,
    Frame,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Snap {
    Ports,
    None,
}

/// Execution selection is explicit; omitted entries preserve the native protocol.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Entry {
    #[default]
    Native,
    Wasm,
}
fn native(entry: &Entry) -> bool {
    *entry == Entry::Native
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Wasm {
    pub module: String,
    pub sha256: String,
    pub abi: String,
}

/// Stroke-only path data on the host's 24-unit icon grid.
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Icon {
    pub paths: Vec<String>,
}

impl Icon {
    pub(super) fn valid(&self) -> bool {
        (1..=8).contains(&self.paths.len())
            && self.paths.iter().map(String::len).sum::<usize>() <= 2048
            && self.paths.iter().all(|path| {
                !path.trim().is_empty()
                    && path.len() <= 512
                    && path
                        .bytes()
                        .all(|byte| b"MmLlHhVvCcSsQqAaZz0123456789.-, ".contains(&byte))
            })
    }
}

/// A toolbar tool (ADR 0046). Pointer input and preview stay in the frontend; the host calls
/// `tool/apply` once per completed gesture.
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Tool {
    #[serde(default, skip_serializing_if = "native")]
    pub entry: Entry,
    pub id: String,
    pub label: String,
    pub icon: Icon,
    pub kind: ToolKind,
    pub snap: Snap,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stroke: bool,
    /// IDs of the addon's settings fields shown in the palette pocket while the tool is active.
    #[serde(default)]
    pub settings: Vec<String>,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct PathEdit {
    pub choice: String,
    pub snap: Snap,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    #[serde(default, skip_serializing_if = "native")]
    pub entry: Entry,
    pub id: String,
    pub label: String,
    pub surface: Surface,
    #[serde(
        default,
        rename = "pathEdit",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional"
    )]
    pub path_edit: Option<PathEdit>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional"
    )]
    pub form: Option<Vec<Field>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional"
    )]
    pub format: Option<Format>,
}

fn optional<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    input: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(input).map(Some)
}

type Locale = BTreeMap<String, String>;

fn locale<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Locale, D::Error> {
    struct Checked;
    impl<'de> Visitor<'de> for Checked {
        type Value = Locale;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a string table with unique keys")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Locale, M::Error> {
            let mut table = Locale::new();
            while let Some((key, value)) = map.next_entry()? {
                if table.insert(key, value).is_some() || table.len() > 128 {
                    return Err(serde::de::Error::custom(
                        "duplicate or excessive string keys",
                    ));
                }
            }
            Ok(table)
        }
    }
    deserializer.deserialize_map(Checked)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Strings {
    #[serde(deserialize_with = "locale")]
    pub en: Locale,
    #[serde(deserialize_with = "locale")]
    pub de: Locale,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional"
    )]
    pub wasm: Option<Wasm>,
    #[serde(default, skip_serializing_if = "native")]
    pub reaction: Entry,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional"
    )]
    pub preview: Option<Entry>,
    manifest_version: u32,
    id: String,
    version: String,
    api_version: String,
    target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entrypoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<super::content::Contents>,
    transport: Transport,
    pub permissions: Vec<Permission>,
    pub commands: Vec<Command>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    pub settings: Vec<Field>,
    pub strings: Strings,
}

#[derive(Debug, PartialEq)]
pub(super) enum Error {
    Invalid,
    NeedsReinstall,
    Incompatible(String),
    MissingFeatures(Vec<String>),
}

pub(super) fn supported_feature(feature: &str) -> bool {
    ["end-caps", "group-stretch", "path-edit-v0"].contains(&feature)
        || (feature == "wasm-v0" && cfg!(feature = "wasm-tier"))
}

pub(super) fn release_version(version: &str) -> bool {
    version.len() <= 32
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(crate) fn identifier(value: &str) -> bool {
    value.len() <= 48
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

impl Addon {
    pub(super) fn is_legacy_release(
        &self,
        version: &str,
        inventory: &PackageManifest,
        release: Option<&PinnedRelease<'_>>,
    ) -> bool {
        self.legacy == Some(version)
            && self.pinned.is_some_and(|pin| {
                pin.version == version
                    && inventory.version == version
                    && inventory.decoder_commit == pin.decoder_commit
                    && release.is_some_and(|candidate| {
                        candidate.sha256.eq_ignore_ascii_case(pin.sha256)
                            && candidate.size == pin.size
                    })
            })
    }

    pub(super) fn permissions(&self) -> &'static [Permission] {
        use Permission::*;
        match self.kind {
            Kind::Import { artwork: true, .. } => {
                &[PickedImportFile, DerivedImportArtwork, PublishImportBatch]
            }
            Kind::Import { .. } => &[PickedImportFile, PublishImportBatch],
            Kind::Recognize { .. } if self.contribution_surfaces.contains(&Surface::Export) => {
                &[SelectedInk, AddPageObjects, ReadPages]
            }
            Kind::Recognize { .. } => &[SelectedInk, AddPageObjects],
            Kind::Objects { permissions, .. } => permissions,
            Kind::Content => &[],
        }
    }

    fn allows_command(&self, command: &Command) -> bool {
        match command.surface {
            Surface::LibraryImport => self.imports_notebooks() && command.id == "import-notebooks",
            Surface::InkSelection => {
                matches!(self.kind, Kind::Recognize { .. }) && command.id == "recognize-math"
            }
            surface => self.contribution_surfaces.contains(&surface),
        }
    }

    pub(super) fn declaration(&self, version: &str) -> Manifest {
        let objects = match self.kind {
            Kind::Objects { tools, .. } => tools,
            _ => &[],
        };
        let (transport, id, surface) = if self.imports_notebooks() {
            (
                Transport::LegacyImportCli,
                "import-notebooks",
                Surface::LibraryImport,
            )
        } else {
            (
                Transport::StdioJsonrpc,
                "recognize-math",
                Surface::InkSelection,
            )
        };
        let strings = |(name, run): (&str, &str)| {
            let mut table = Locale::from([("name".into(), name.into())]);
            if !self.is_content() {
                table.insert("run".into(), run.into());
            }
            table
        };
        Manifest {
            wasm: None,
            reaction: Entry::Native,
            preview: None,
            manifest_version: 1,
            id: self.id.into(),
            version: version.into(),
            api_version: API.into(),
            target: TARGET.into(),
            requires: if self.id == catalog::CONNECTORS.id {
                vec!["path-edit-v0".into()]
            } else {
                vec![]
            },
            entrypoint: (!self.is_content()).then(|| self.executable()),
            contents: None,
            transport: if self.is_content() {
                Transport::Content
            } else {
                transport
            },
            permissions: self.permissions().to_vec(),
            commands: if self.is_content() || !objects.is_empty() {
                vec![]
            } else {
                vec![Command {
                    entry: Entry::Native,
                    path_edit: None,
                    id: id.into(),
                    label: "run".into(),
                    surface,
                    form: None,
                    format: None,
                }]
            },
            tools: objects
                .iter()
                .map(|(id, kind)| Tool {
                    entry: Entry::Native,
                    id: (*id).into(),
                    label: "run".into(),
                    stroke: false,
                    icon: Icon {
                        paths: vec!["M4 12h16".into()],
                    },
                    kind: *kind,
                    snap: Snap::Ports,
                    settings: vec![],
                })
                .collect(),
            settings: vec![],
            strings: Strings {
                en: strings(self.en),
                de: strings(self.de),
            },
        }
    }
}

impl Wasm {
    pub(super) fn valid(&self) -> bool {
        self.abi == "goodtype-wasm-v0"
            && self.module == "module.wasm"
            && update::is_hash(&self.sha256, 64)
            && !self.sha256.bytes().any(|b| b.is_ascii_uppercase())
    }
}
impl Manifest {
    fn valid_wasm(&self) -> bool {
        let used = self.reaction == Entry::Wasm
            || self.preview == Some(Entry::Wasm)
            || self.commands.iter().any(|c| c.entry == Entry::Wasm)
            || self.tools.iter().any(|t| t.entry == Entry::Wasm);
        if let Some(wasm) = &self.wasm {
            if !used
                || !self.requires.iter().any(|f| f == "wasm-v0")
                || !wasm.valid()
                || !matches!(self.transport, Transport::StdioJsonrpc)
                || self.permissions.iter().any(|p| {
                    !matches!(
                        p,
                        Permission::OwnObjects
                            | Permission::ReactToChanges
                            | Permission::AddPageObjects
                    )
                })
            {
                return false;
            }
        } else if used {
            return false;
        }
        self.target != WASM_TARGET
            || (self.entrypoint.is_none()
                && self.commands.iter().all(|c| c.entry == Entry::Wasm)
                && self.tools.iter().all(|t| t.entry == Entry::Wasm)
                && (!self.has(Permission::ReactToChanges) || self.reaction == Entry::Wasm))
    }
    pub(super) fn target_is_wasm(&self) -> bool {
        self.target == WASM_TARGET
    }
    pub(super) fn wasm_entry(&self, method: &str, command: &str) -> bool {
        match method {
            "page/changed" => self.reaction == Entry::Wasm,
            "tool/apply" => self
                .tools
                .iter()
                .any(|t| t.id == command && t.entry == Entry::Wasm),
            _ => self
                .commands
                .iter()
                .any(|c| c.id == command && c.entry == Entry::Wasm),
        }
    }

    pub(super) fn selection_labels(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.commands
            .iter()
            .filter(|command| self.supports_selection(&command.id))
            .map(|command| {
                let en = &self.strings.en[&command.label];
                (
                    command.id.as_str(),
                    en.as_str(),
                    self.strings.de.get(&command.label).unwrap_or(en).as_str(),
                )
            })
    }

    pub(super) fn supports_selection(&self, id: &str) -> bool {
        self.transport == Transport::StdioJsonrpc
            && self
                .commands
                .iter()
                .any(|command| command.id == id && command.surface == Surface::InkSelection)
    }

    pub(super) fn path_edit(&self) -> bool {
        self.commands
            .iter()
            .any(|command| command.path_edit.is_some())
    }

    pub(super) fn uses_session(&self) -> bool {
        self.transport == Transport::StdioJsonrpc
    }

    pub(super) fn has(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }

    pub(super) fn same_scope(&self, other: &Self) -> bool {
        self.wasm == other.wasm
            && self.reaction == other.reaction
            && self.preview == other.preview
            && self.transport == other.transport
            && self.permissions.len() == other.permissions.len()
            && self
                .permissions
                .iter()
                .all(|permission| other.permissions.contains(permission))
            && self
                .tools
                .iter()
                .map(|tool| (&tool.id, tool.kind, tool.entry))
                .eq(other
                    .tools
                    .iter()
                    .map(|tool| (&tool.id, tool.kind, tool.entry)))
            && self
                .commands
                .iter()
                .map(|command| {
                    (
                        &command.id,
                        command.surface,
                        command.format.as_ref().is_some_and(|f| f.assets),
                        &command.path_edit,
                        command.entry,
                    )
                })
                .eq(other.commands.iter().map(|command| {
                    (
                        &command.id,
                        command.surface,
                        command.format.as_ref().is_some_and(|f| f.assets),
                        &command.path_edit,
                        command.entry,
                    )
                }))
    }
}

pub(super) fn parse(bytes: &[u8], addon: &Addon, version: &str) -> Result<Manifest, Error> {
    if bytes.len() > MAX_BYTES || std::str::from_utf8(bytes).is_err() {
        return Err(Error::Invalid);
    }
    let super::session::Strict(shape) =
        serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if !shape.is_object()
        || !shape["strings"].is_object()
        || !shape["commands"]
            .as_array()
            .is_some_and(|commands| commands.iter().all(serde_json::Value::is_object))
    {
        return Err(Error::Invalid);
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    let ceiling = addon.declaration(version);
    let mut permissions = HashSet::new();
    let mut commands = HashSet::new();
    let mut tools = HashSet::new();
    let allowed_tools: &[(&str, ToolKind)] = match addon.kind {
        Kind::Objects { tools, .. } => tools,
        _ => &[],
    };
    if manifest.manifest_version != 1
        || manifest.id != addon.id
        || manifest.version != version
        || !release_version(version)
        || (!manifest.uses_session() && !manifest.settings.is_empty())
        || !valid_fields(&manifest.settings, &manifest.strings.en, true)
        || !(manifest.target == TARGET
            || (manifest.target == WASM_TARGET && manifest.wasm.is_some()))
        || (manifest.entrypoint != ceiling.entrypoint
            && !(manifest.target == WASM_TARGET && manifest.entrypoint.is_none()))
        || !manifest.valid_wasm()
        || manifest.transport != ceiling.transport
        || manifest.permissions.iter().any(|permission| {
            !ceiling.permissions.contains(permission) || !permissions.insert(permission)
        })
        || if addon.is_content() {
            shape.get("entrypoint").is_some()
                || !manifest.commands.is_empty()
                || !manifest
                    .contents
                    .as_ref()
                    .is_some_and(super::content::Contents::valid)
        } else {
            shape.get("contents").is_some()
                || manifest.commands.len() > 16
                || (manifest.commands.is_empty() && manifest.tools.is_empty())
        }
        || manifest.tools.len() > 8
        || (!manifest.tools.is_empty() && !permissions.contains(&Permission::OwnObjects))
        || manifest.tools.iter().any(|tool| {
            !tools.insert(&tool.id)
                || !identifier(&tool.id)
                || !manifest.strings.en.contains_key(&tool.label)
                || !tool.icon.valid()
                || !allowed_tools.contains(&(tool.id.as_str(), tool.kind))
                || tool.settings.len() > 16
                || {
                    let mut seen = HashSet::new();
                    tool.settings.iter().any(|id| {
                        !seen.insert(id) || !manifest.settings.iter().any(|field| field.id() == id)
                    })
                }
        })
        || manifest.commands.iter().any(|command| {
            !commands.insert(&command.id)
                || !manifest.strings.en.contains_key(&command.label)
                || !identifier(&command.id)
                || !addon.allows_command(command)
                || command.path_edit.as_ref().is_some_and(|edit| {
                    !addon.allows_path_edit(&command.id)
                        || command.surface != Surface::OwnObject
                        || edit.snap != Snap::Ports
                        || !manifest.settings.iter().any(
                            |field| matches!(field, Field::Choice { id, .. } if id == &edit.choice),
                        )
                })
                || match command.surface {
                    Surface::LibraryImport => command.form.is_some() || command.format.is_some(),
                    Surface::InkSelection => {
                        command.form.is_some()
                            || command.format.is_some()
                            || !permissions.contains(&Permission::SelectedInk)
                            || !permissions.contains(&Permission::AddPageObjects)
                    }
                    Surface::InsertForm | Surface::PageGenerator => {
                        command.format.is_some()
                            || !command
                                .form
                                .as_ref()
                                .is_some_and(|form| valid_fields(form, &manifest.strings.en, false))
                            || !permissions.contains(&Permission::AddPageObjects)
                    }
                    Surface::Export => {
                        command.form.is_some()
                            || !command
                                .format
                                .as_ref()
                                .is_some_and(|format| format.valid(&manifest.strings.en))
                            || !permissions.contains(&Permission::ReadPages)
                    }
                    Surface::OwnObject => {
                        command.form.is_some()
                            || command.format.is_some()
                            || !permissions.contains(&Permission::OwnObjects)
                    }
                }
        })
        || !manifest.strings.en.contains_key("name")
        || !manifest.strings.en.keys().eq(manifest.strings.de.keys())
        || [&manifest.strings.en, &manifest.strings.de]
            .into_iter()
            .flatten()
            .any(|(key, text)| {
                !identifier(key)
                    || !(1..=256).contains(&text.chars().count())
                    || text.contains(['<', '>'])
                    || text.contains("{{")
                    || text.contains("}}")
                    || text.chars().any(char::is_control)
            })
    {
        return Err(Error::Invalid);
    }
    if manifest.requires.len() > 16
        || manifest.requires.iter().any(|feature| !identifier(feature))
        || manifest
            .requires
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != manifest.requires.len()
    {
        return Err(Error::Invalid);
    }
    if manifest.api_version != API {
        return Err(Error::Incompatible(
            manifest
                .api_version
                .strip_prefix("goodtype-api ")
                .filter(|version| {
                    version.len() <= 32
                        && version
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || byte == b'.')
                })
                .map_or_else(String::new, |_| manifest.api_version.clone()),
        ));
    }
    let mut missing: Vec<String> = manifest
        .requires
        .iter()
        .filter(|feature| !supported_feature(feature))
        .cloned()
        .collect();
    if addon.id == catalog::CONNECTORS.id
        && !manifest
            .requires
            .iter()
            .any(|feature| feature == "path-edit-v0")
    {
        missing.push("path-edit-v0".into());
    }
    if !missing.is_empty() {
        return Err(Error::MissingFeatures(missing));
    }
    Ok(manifest)
}

pub(super) fn load(
    addon: &Addon,
    path: &Path,
    version: &str,
    inventory: &PackageManifest,
    release: Option<&PinnedRelease<'_>>,
) -> Result<Manifest, Error> {
    let file = path.join(FILE);
    match fs::symlink_metadata(&file) {
        Ok(_) => {
            let bytes = read_bounded(&file, MAX_BYTES).map_err(|_| Error::Invalid)?;
            let declaration = parse(&bytes, addon, version)?;
            if declaration.target != inventory.target {
                return Err(Error::Invalid);
            }
            Ok(declaration)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !addon.is_legacy_release(version, inventory, release)
                || inventory.notebook_schema_version != goodtype_core::BINARY_INK_SCHEMA_VERSION
            {
                return Err(Error::Invalid);
            }
            Ok(addon.declaration(version))
        }
        Err(_) => Err(Error::Invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn wasm_manifest_requires_a_valid_profile_and_explicit_gate() {
        let addon = &catalog::CONNECTORS;
        let mut value = serde_json::to_value(addon.declaration("1.2.3")).unwrap();
        value["requires"] = json!(["wasm-v0", "path-edit-v0"]);
        value["target"] = json!(WASM_TARGET);
        value.as_object_mut().unwrap().remove("entrypoint");
        value["wasm"] =
            json!({"module":"module.wasm","sha256":"a".repeat(64),"abi":"goodtype-wasm-v0"});
        value["reaction"] = json!("wasm");
        value["preview"] = json!("wasm");
        for tool in value["tools"].as_array_mut().unwrap() {
            tool["entry"] = json!("wasm");
        }
        let parse_value = |v: &Value| parse(&serde_json::to_vec(v).unwrap(), addon, "1.2.3");
        assert_eq!(parse_value(&value).is_ok(), cfg!(feature = "wasm-tier"));
        if !cfg!(feature = "wasm-tier") {
            assert!(
                matches!(parse_value(&value), Err(Error::MissingFeatures(features)) if features == ["wasm-v0"])
            );
        }
        for (key, bad) in [
            ("module", json!("../module.wasm")),
            ("sha256", json!("bad")),
            ("abi", json!("other")),
        ] {
            let mut broken = value.clone();
            broken["wasm"][key] = bad;
            assert!(matches!(parse_value(&broken), Err(Error::Invalid)));
        }
        let mut missing = value.clone();
        missing["requires"] = json!([]);
        assert!(matches!(parse_value(&missing), Err(Error::Invalid)));
        value["tools"][0]["entry"] = json!("native");
        assert!(matches!(parse_value(&value), Err(Error::Invalid)));
    }

    #[test]
    fn shared_requirement_fixtures_use_real_loader() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/addon-requires/manifests.json"
        ))
        .unwrap();
        let home = tempfile::tempdir().unwrap();
        let inventory = PackageManifest {
            version: "1.2.3".into(),
            target: TARGET.into(),
            decoder_commit: String::new(),
            protocol_version: 1,
            notebook_schema_version: 0,
            files: vec![],
        };
        for fixture in fixtures.as_array().unwrap() {
            let value = &fixture["manifest"];
            let addon = CATALOG
                .iter()
                .find(|addon| addon.id == value["id"].as_str().unwrap())
                .unwrap();
            fs::write(home.path().join(FILE), serde_json::to_vec(value).unwrap()).unwrap();
            let outcome = match load(addon, home.path(), "1.2.3", &inventory, None) {
                Ok(_) => "accepted".into(),
                Err(Error::Invalid) => "invalid".into(),
                Err(Error::MissingFeatures(features)) => format!("missing:{}", features.join(",")),
                Err(other) => panic!("unexpected {other:?}"),
            };
            assert_eq!(
                outcome,
                fixture["expected"].as_str().unwrap(),
                "{}",
                fixture["name"]
            );
        }
    }

    #[test]
    fn documented_example_and_catalog_fixtures_round_trip() {
        let doc = include_str!("../../../../../formats/addon-api-v0.md");
        let example = doc
            .split("```json")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let parsed = parse(example.as_bytes(), &catalog::ONENOTE, "0.1.1").unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::from_str::<Value>(example).unwrap()
        );
        for &addon in CATALOG.iter().filter(|addon| !addon.is_content()) {
            let bytes = serde_json::to_vec(&addon.declaration("1.2.3")).unwrap();
            let parsed = parse(&bytes, addon, "1.2.3").unwrap();
            assert_eq!(serde_json::to_vec(&parsed).unwrap(), bytes);
        }
    }

    #[test]
    fn rejects_malformed_and_out_of_contract_declarations() {
        let addon = &catalog::GOODNOTES;
        let declaration = serde_json::to_value(addon.declaration("1.2.3")).unwrap();
        for (key, value) in [
            ("manifestVersion", json!(2)),
            ("id", json!("other")),
            ("version", json!("1.2.4")),
            ("target", json!("other")),
            ("entrypoint", json!("../goodtype-goodnotes-import.exe")),
            ("transport", json!("stdio-jsonrpc")),
            ("permissions", json!(["selected-ink"])),
            (
                "permissions",
                json!(["picked-import-file", "picked-import-file"]),
            ),
            ("commands", json!([])),
            (
                "commands",
                json!([declaration["commands"][0], declaration["commands"][0]]),
            ),
            (
                "commands",
                json!([{"id":"import-notebooks","surface":"ink-selection","label":"run"}]),
            ),
            (
                "commands",
                json!([{"id":"import-notebooks","surface":"library-import","label":"missing"}]),
            ),
            (
                "commands",
                json!([["import-notebooks", "run", "library-import"]]),
            ),
            (
                "settings",
                json!([{"id":"test","label":"run","type":"choice","default":"invalid","choices":[{"value":"one","label":"run"},{"value":"two","label":"run"}]}]),
            ),
            (
                "settings",
                json!([{"id":"test","label":"run","type":"toggle","default":false}]),
            ),
            (
                "strings",
                json!({"en":{"name":"Name","run":"Run"},"de":{"name":"Name"}}),
            ),
            ("strings", json!({"en":{"run":"Run"},"de":{"run":"Start"}})),
            ("arguments", json!([])),
        ] {
            let mut bad = declaration.clone();
            bad[key] = value;
            assert!(
                matches!(
                    parse(&serde_json::to_vec(&bad).unwrap(), addon, "1.2.3"),
                    Err(Error::Invalid)
                ),
                "{key}: {bad}"
            );
        }
        for text in ["", "<script>", "{{name}}", "\n", &"a".repeat(257)] {
            let mut bad = declaration.clone();
            bad["strings"]["en"]["name"] = json!(text);
            assert!(parse(&serde_json::to_vec(&bad).unwrap(), addon, "1.2.3").is_err());
        }
        for bytes in [b"{".as_slice(), b"\xff", &[b' '; MAX_BYTES + 1]] {
            assert!(parse(bytes, addon, "1.2.3").is_err());
        }
        let valid = serde_json::to_string(&addon.declaration("1.2.3")).unwrap();
        for bad in [
            valid.replace(
                "\"manifestVersion\":1",
                "\"manifestVersion\":1,\"manifestVersion\":1",
            ),
            valid.replace("\"label\":\"run\"", "\"label\":\"run\",\"label\":\"run\""),
            valid.replace("\"name\":", "\"name\":\"duplicate\",\"name\":"),
        ] {
            assert!(parse(bad.as_bytes(), addon, "1.2.3").is_err());
        }
        assert!(matches!(
            parse(valid.replace(API, "goodtype-api 0.2").as_bytes(), addon, "1.2.3"),
            Err(Error::Incompatible(version)) if version == "goodtype-api 0.2"
        ));
    }

    #[test]
    fn only_exact_pinned_importers_get_legacy_declarations() {
        let home = tempfile::tempdir().unwrap();
        for addon in [&catalog::GOODNOTES, &catalog::ONENOTE] {
            let pin = addon.pinned.unwrap();
            let inventory = PackageManifest {
                version: pin.version.into(),
                target: TARGET.into(),
                decoder_commit: pin.decoder_commit.into(),
                protocol_version: 1,
                notebook_schema_version: goodtype_core::BINARY_INK_SCHEMA_VERSION,
                files: vec![],
            };
            assert!(load(addon, home.path(), pin.version, &inventory, Some(&pin)).is_ok());
            assert!(load(addon, home.path(), pin.version, &inventory, None).is_err());
            let wrong_archive = PinnedRelease {
                wasm: None,
                sha256: "different",
                ..pin
            };
            assert!(
                load(
                    addon,
                    home.path(),
                    pin.version,
                    &inventory,
                    Some(&wrong_archive)
                )
                .is_err()
            );
            assert!(load(addon, home.path(), "9.9.9", &inventory, None).is_err());
            assert!(load(&catalog::MATH, home.path(), pin.version, &inventory, None).is_err());
        }
    }
}
