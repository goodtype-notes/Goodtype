use super::*;
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
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Transport {
    LegacyImportCli,
    StdioJsonrpc,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Surface {
    LibraryImport,
    InkSelection,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Command {
    id: String,
    label: String,
    surface: Surface,
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
    manifest_version: u32,
    id: String,
    version: String,
    api_version: String,
    target: String,
    entrypoint: String,
    transport: Transport,
    pub permissions: Vec<Permission>,
    commands: Vec<Command>,
    settings: Vec<serde_json::Value>,
    pub strings: Strings,
}

#[derive(Debug, PartialEq)]
pub(super) enum Error {
    Invalid,
    NeedsReinstall,
    Incompatible(String),
}

pub(super) fn release_version(version: &str) -> bool {
    version.len() <= 32
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(super) fn identifier(value: &str) -> bool {
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
            Kind::Recognize { .. } => &[SelectedInk, AddPageObjects],
        }
    }

    pub(super) fn declaration(&self, version: &str) -> Manifest {
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
            [("name".into(), name.into()), ("run".into(), run.into())].into()
        };
        Manifest {
            manifest_version: 1,
            id: self.id.into(),
            version: version.into(),
            api_version: API.into(),
            target: TARGET.into(),
            entrypoint: self.executable(),
            transport,
            permissions: self.permissions().to_vec(),
            commands: vec![Command {
                id: id.into(),
                label: "run".into(),
                surface,
            }],
            settings: vec![],
            strings: Strings {
                en: strings(self.en),
                de: strings(self.de),
            },
        }
    }
}

impl Manifest {
    pub(super) fn same_scope(&self, other: &Self) -> bool {
        self.transport == other.transport
            && self.permissions.len() == other.permissions.len()
            && self
                .permissions
                .iter()
                .all(|permission| other.permissions.contains(permission))
            && self
                .commands
                .iter()
                .map(|command| (&command.id, command.surface))
                .eq(other
                    .commands
                    .iter()
                    .map(|command| (&command.id, command.surface)))
    }
}

pub(super) fn parse(bytes: &[u8], addon: &Addon, version: &str) -> Result<Manifest, Error> {
    if bytes.len() > MAX_BYTES || std::str::from_utf8(bytes).is_err() {
        return Err(Error::Invalid);
    }
    let shape: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
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
    if manifest.manifest_version != 1
        || manifest.id != addon.id
        || manifest.version != version
        || !release_version(version)
        || !manifest.settings.is_empty()
        || manifest.target != TARGET
        || manifest.entrypoint != ceiling.entrypoint
        || manifest.transport != ceiling.transport
        || manifest.permissions.iter().any(|permission| {
            !ceiling.permissions.contains(permission) || !permissions.insert(permission)
        })
        || !(1..=16).contains(&manifest.commands.len())
        || manifest.commands.iter().any(|command| {
            !commands.insert(&command.id)
                || !manifest.strings.en.contains_key(&command.label)
                || !ceiling
                    .commands
                    .iter()
                    .any(|allowed| allowed.id == command.id && allowed.surface == command.surface)
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
            parse(&bytes, addon, version)
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
        for &addon in CATALOG {
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
