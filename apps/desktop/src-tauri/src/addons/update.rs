//! Explicit stable-channel discovery. Only signed metadata can authorize a new executable.
use std::io::Read;

use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

use super::{Addon, MAX_PACKAGE_BYTES, PROTOCOL_VERSION, PinnedRelease, TARGET, github_asset};

const MAX_DESCRIPTOR: u64 = 64 * 1024;

#[derive(Clone, Debug)]
pub(super) struct VerifiedUpdate {
    pub version: String,
    pub size: u64,
    url: String,
    sha256: String,
    decoder_commit: String,
}

impl VerifiedUpdate {
    pub fn pin(&self) -> PinnedRelease<'_> {
        PinnedRelease {
            version: &self.version,
            size: self.size,
            url: &self.url,
            sha256: &self.sha256,
            decoder_commit: &self.decoder_commit,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    version: String,
    protocol_version: Option<u64>,
    notebook_schema_version: Option<u32>,
    target: String,
    source_dirty: bool,
    source_commit: String,
    decoder_commit: String,
    archive: Archive,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Archive {
    file_name: String,
    size_bytes: u64,
    sha256: String,
    url: String,
}

fn stable_version(version: &str) -> Result<[u32; 3], String> {
    let parts = version
        .split('.')
        .map(|part| {
            if part.is_empty()
                || part.len() > 9
                || !part.bytes().all(|c| c.is_ascii_digit())
                || part.len() > 1 && part.starts_with('0')
            {
                return Err("not a stable importer version".to_owned());
            }
            part.parse::<u32>()
                .map_err(|_| "invalid importer version".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    parts
        .try_into()
        .map_err(|_| "not a three-part importer version".into())
}

#[cfg(test)]
fn require_newer(version: &str, installed: &str) -> Result<(), String> {
    if stable_version(version)? <= stable_version(installed)? {
        return Err("a newer or equal importer version is already installed".into());
    }
    Ok(())
}

pub(super) fn is_hash(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn verify_signature(
    bytes: &[u8],
    signature: &[u8],
    key: &[u8; 32],
) -> Result<(), String> {
    VerifyingKey::from_bytes(key)
        .map_err(|_| "invalid release verification key")?
        .verify_strict(
            bytes,
            &Signature::from_slice(signature).map_err(|_| "invalid release signature length")?,
        )
        .map_err(|_| "invalid release signature".into())
}

/// Authenticates `bytes` with `key` and checks they describe a compatible, clean, stable release
/// of `tool` from its own repository.
fn verify(
    tool: &Addon,
    bytes: &[u8],
    signature: &[u8],
    key: &[u8; 32],
) -> Result<VerifiedUpdate, String> {
    if bytes.len() as u64 > MAX_DESCRIPTOR {
        return Err("importer update metadata is too large".into());
    }
    verify_signature(bytes, signature, key)?;
    let descriptor: Descriptor = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid importer update metadata ({error})"))?;
    stable_version(&descriptor.version)?;
    if descriptor.source_dirty
        || !is_hash(&descriptor.source_commit, 40)
        || if tool.imports_notebooks() {
            !is_hash(&descriptor.decoder_commit, 40)
        } else {
            !descriptor.decoder_commit.is_empty()
        }
        || !is_hash(&descriptor.archive.sha256, 64)
    {
        return Err("importer update is not a clean, identified release".into());
    }
    // Only the importer CLI protocol carries versions here. Importers may write any notebook
    // schema this Goodtype reads; the core reader validates every imported notebook before it
    // is published. Session addons are bound by their `goodtype-api` manifest and handshake.
    let compatible = if tool.imports_notebooks() {
        descriptor.protocol_version == Some(PROTOCOL_VERSION)
            && descriptor
                .notebook_schema_version
                .is_some_and(super::importable_schema)
    } else if tool.is_content() {
        descriptor.protocol_version.is_none() && descriptor.notebook_schema_version.is_none()
    } else {
        true
    };
    if !compatible || descriptor.target != TARGET {
        return Err("the newest importer requires a different Goodtype version; the installed version was kept".into());
    }
    let name = format!("{}-{}-windows-x64.zip", tool.package, descriptor.version);
    if descriptor.archive.file_name != name
        || descriptor.archive.url
            != format!("{}/v{}/{name}", tool.release_url(), descriptor.version)
        || descriptor.archive.size_bytes == 0
        || descriptor.archive.size_bytes > MAX_PACKAGE_BYTES
    {
        return Err("unexpected importer update package URL or size".into());
    }
    Ok(VerifiedUpdate {
        version: descriptor.version,
        size: descriptor.archive.size_bytes,
        url: descriptor.archive.url,
        sha256: descriptor.archive.sha256,
        decoder_commit: descriptor.decoder_commit,
    })
}

pub(super) fn fetch(
    url: &str,
    limit: u64,
    allowed: impl Fn(&str, bool) -> bool,
) -> Result<Vec<u8>, super::registry::Problem> {
    let mut bytes = Vec::new();
    github_asset(url, allowed)?
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| super::registry::Problem::Offline)?;
    if bytes.len() as u64 > limit {
        return Err(super::registry::Problem::Invalid);
    }
    Ok(bytes)
}

/// The signed descriptor each kind of addon release publishes beside its ZIP. Math's published
/// releases name theirs `recognizer-update.json`; looking for the importer name broke installing it.
pub(super) fn descriptor_name(tool: &Addon) -> &'static str {
    match tool.kind {
        super::Kind::Content => "content-update.json",
        super::Kind::Import { .. } => "importer-update.json",
        super::Kind::Recognize { .. } => "recognizer-update.json",
        super::Kind::Objects { .. } => "addon-update.json",
    }
}

pub(super) fn descriptor_for(
    tool: &Addon,
    release: &super::registry::Release,
) -> Result<VerifiedUpdate, String> {
    let (folder, _) = release
        .url
        .rsplit_once('/')
        .ok_or("invalid addon asset URL")?;
    let allowed = |url: &str, redirected| super::download_url_allowed(tool, url, redirected);
    let name = descriptor_name(tool);
    let bytes = fetch(&format!("{folder}/{name}"), MAX_DESCRIPTOR, allowed)
        .map_err(|problem| format!("cannot read the addon descriptor ({problem:?})"))?;
    let signature = fetch(&format!("{folder}/{name}.sig"), 64, allowed)
        .map_err(|problem| format!("cannot read the addon signature ({problem:?})"))?;
    let verified = verify(
        tool,
        &bytes,
        &signature,
        &tool.public_key.ok_or("no signed update key is available")?,
    )?;
    if verified.version != release.version
        || verified.url != release.url
        || verified.size != release.size
        || !verified.sha256.eq_ignore_ascii_case(&release.sha256)
    {
        return Err("signed addon descriptor differs from its index entry".into());
    }
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{GOODNOTES, ONENOTE};
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    #[test]
    fn each_kind_reads_the_descriptor_its_releases_publish() {
        use super::super::catalog::{CONNECTORS, ENGINEERING, MATH};
        assert_eq!(descriptor_name(&GOODNOTES), "importer-update.json");
        assert_eq!(descriptor_name(&MATH), "recognizer-update.json");
        assert_eq!(descriptor_name(&ENGINEERING), "content-update.json");
        assert_eq!(descriptor_name(&CONNECTORS), "addon-update.json");
    }

    fn descriptor(tool: &Addon) -> serde_json::Value {
        let name = format!("{}-0.1.1-windows-x64.zip", tool.package);
        json!({"version":"0.1.1", "protocolVersion":1, "notebookSchemaVersion":goodtype_core::SCHEMA_VERSION,
            "target":TARGET, "sourceDirty":false, "sourceCommit":"1".repeat(40), "decoderCommit":"2".repeat(40),
            "archive":{"fileName":name, "sizeBytes":1234,
                "sha256":"3".repeat(64), "url":format!("{}/v0.1.1/{name}", tool.release_url())}})
    }

    #[test]
    fn authenticated_update_rejects_tampering_and_wrong_keys() {
        let key = SigningKey::from_bytes(&[42; 32]);
        for tool in [&GOODNOTES, &ONENOTE] {
            let bytes = serde_json::to_vec(&descriptor(tool)).unwrap();
            let signature = key.sign(&bytes).to_bytes();
            let ours = key.verifying_key();
            let verified = verify(tool, &bytes, &signature, ours.as_bytes()).unwrap();
            assert_eq!(verified.pin().version, "0.1.1");
            let mut changed = bytes.clone();
            changed.push(b' ');
            assert!(verify(tool, &changed, &signature, ours.as_bytes()).is_err());
            assert!(verify(tool, &bytes, &signature, &tool.public_key.unwrap()).is_err());
            assert!(verify(tool, &bytes, &[0; 63], ours.as_bytes()).is_err());
        }
    }

    #[test]
    fn one_importers_release_never_authorizes_the_other() {
        assert_ne!(GOODNOTES.public_key, ONENOTE.public_key);
        let key = SigningKey::from_bytes(&[42; 32]);
        for (named, used) in [(&GOODNOTES, &ONENOTE), (&ONENOTE, &GOODNOTES)] {
            // Even correctly signed, a descriptor for one importer's package and repository is
            // refused when checked as the other's.
            let bytes = serde_json::to_vec(&descriptor(named)).unwrap();
            let signature = key.sign(&bytes).to_bytes();
            assert!(verify(used, &bytes, &signature, key.verifying_key().as_bytes()).is_err());
        }
    }

    #[test]
    fn signed_metadata_still_needs_compatible_clean_bounded_assets() {
        let key = SigningKey::from_bytes(&[42; 32]);
        for tool in [&GOODNOTES, &ONENOTE] {
            for (pointer, value) in [
                ("/protocolVersion", json!(2)),
                ("/notebookSchemaVersion", json!(999)),
                ("/target", json!("wrong-target")),
                ("/sourceDirty", json!(true)),
                ("/sourceCommit", json!("bad")),
                ("/version", json!("0.1.2-rc.1")),
                ("/archive/url", json!("https://evil.invalid/tool.zip")),
                ("/archive/sha256", json!("bad")),
                ("/archive/sizeBytes", json!(MAX_PACKAGE_BYTES + 1)),
            ] {
                let mut data = descriptor(tool);
                *data.pointer_mut(pointer).unwrap() = value;
                let bytes = serde_json::to_vec(&data).unwrap();
                let signature = key.sign(&bytes).to_bytes();
                assert!(
                    verify(tool, &bytes, &signature, key.verifying_key().as_bytes()).is_err(),
                    "{} {pointer}",
                    tool.id
                );
            }
        }
    }

    #[test]
    fn content_descriptors_omit_importer_fields() {
        use super::super::catalog::ENGINEERING;
        let key = SigningKey::from_bytes(&[42; 32]);
        let check = |tool: &Addon, value: &serde_json::Value| {
            let bytes = serde_json::to_vec(value).unwrap();
            verify(
                tool,
                &bytes,
                &key.sign(&bytes).to_bytes(),
                key.verifying_key().as_bytes(),
            )
        };
        // The shape the content packs' sign_release.py writes.
        let mut value = descriptor(&ENGINEERING);
        let object = value.as_object_mut().unwrap();
        object.remove("protocolVersion");
        object.remove("notebookSchemaVersion");
        value["decoderCommit"] = json!("");
        assert_eq!(check(&ENGINEERING, &value).unwrap().version, "0.1.1");
        assert!(check(&ENGINEERING, &descriptor(&ENGINEERING)).is_err());
        value["decoderCommit"] = json!("2".repeat(40));
        assert!(check(&ENGINEERING, &value).is_err());
        let mut importer = descriptor(&GOODNOTES);
        importer.as_object_mut().unwrap().remove("protocolVersion");
        assert!(check(&GOODNOTES, &importer).is_err());
    }

    #[test]
    fn descriptor_versions_follow_each_addon_kind() {
        use super::super::catalog::{CONNECTORS, MATH};
        let key = SigningKey::from_bytes(&[42; 32]);
        let check = |tool: &Addon, value: &serde_json::Value| {
            let bytes = serde_json::to_vec(value).unwrap();
            verify(
                tool,
                &bytes,
                &key.sign(&bytes).to_bytes(),
                key.verifying_key().as_bytes(),
            )
        };
        // Importers: protocol 1, any notebook schema the core reads, an identified decoder.
        let mut value = descriptor(&GOODNOTES);
        for schema in [
            goodtype_core::BINARY_INK_SCHEMA_VERSION,
            goodtype_core::SCHEMA_VERSION,
        ] {
            value["notebookSchemaVersion"] = json!(schema);
            assert!(check(&GOODNOTES, &value).is_ok(), "schema {schema}");
        }
        for (field, bad) in [
            ("notebookSchemaVersion", json!(0)),
            (
                "notebookSchemaVersion",
                json!(goodtype_core::SCHEMA_VERSION + 1),
            ),
            ("notebookSchemaVersion", json!(null)),
            ("protocolVersion", json!(2)),
            ("decoderCommit", json!("")),
        ] {
            let mut value = descriptor(&GOODNOTES);
            value[field] = bad;
            assert!(check(&GOODNOTES, &value).is_err(), "{field}");
        }
        // Session addons, as published by math 0.1.1 and connectors 0.1.0: no decoder, and the
        // importer versions carry no meaning (the goodtype-api manifest decides).
        for (tool, versions) in [
            (
                &MATH,
                json!({"protocolVersion": 1, "notebookSchemaVersion": 0}),
            ),
            (&CONNECTORS, json!({})),
        ] {
            let mut value = descriptor(tool);
            let object = value.as_object_mut().unwrap();
            object.remove("protocolVersion");
            object.remove("notebookSchemaVersion");
            object.extend(versions.as_object().unwrap().clone());
            object.insert("decoderCommit".into(), json!(""));
            assert!(check(tool, &value).is_ok(), "{}", tool.id);
            value["decoderCommit"] = json!("2".repeat(40));
            assert!(check(tool, &value).is_err(), "{} decoder", tool.id);
        }
    }

    #[test]
    fn versions_are_numeric_stable_and_cannot_downgrade() {
        assert!(require_newer("0.1.10", "0.1.9").is_ok());
        assert!(require_newer("0.1.1", "0.1.1").is_err());
        assert!(require_newer("0.1.1", "0.2.0").is_err());
        for version in [
            "../1",
            "v1.2.3",
            "1.2",
            "01.2.3",
            "1.2.3-rc.1",
            "1.2.3/../4",
            "9999999999.1.2",
        ] {
            assert!(stable_version(version).is_err(), "{version}");
        }
    }

    fn tool_from_env() -> &'static Addon {
        match std::env::var("IMPORTER_TOOL").as_deref() {
            Ok("onenote") => &ONENOTE,
            _ => &GOODNOTES,
        }
    }

    /// `IMPORTER_TOOL=onenote|goodnotes IMPORTER_UPDATE_FIXTURE=<downloaded release assets>`
    #[test]
    #[ignore = "needs a real signed release bundle in IMPORTER_UPDATE_FIXTURE"]
    fn real_signed_release_installs_with_the_production_key() {
        use super::super::{InstallDir, install_archive, probe_tool, verify_package};
        use crate::store::read_bounded;
        let tool = tool_from_env();
        let root = std::path::PathBuf::from(std::env::var_os("IMPORTER_UPDATE_FIXTURE").unwrap());
        let bytes =
            read_bounded(&root.join("importer-update.json"), MAX_DESCRIPTOR as usize).unwrap();
        let signature = read_bounded(&root.join("importer-update.json.sig"), 64).unwrap();
        let release = verify(tool, &bytes, &signature, &tool.public_key.unwrap()).unwrap();
        let archive = root.join(format!(
            "{}-{}-windows-x64.zip",
            tool.package, release.version
        ));
        let temporary = tempfile::tempdir().unwrap();
        let dir = InstallDir(temporary.path().to_path_buf(), tool);
        let previous = temporary.path().join("versions/0.0.9");
        std::fs::create_dir_all(&previous).unwrap();
        std::fs::write(previous.join("placeholder"), b"retired importer fixture").unwrap();
        let installed = install_archive(&dir, &archive, &release.pin(), &|path| {
            probe_tool(tool, path)
        })
        .unwrap();
        assert_eq!(installed, release.version);
        assert!(!previous.exists());
        let (_, active) = dir.active().unwrap();
        verify_package(tool, &active).unwrap();
        println!(
            "Verified the {} signature, installed {installed}, passed its self-check and removed the inactive version.",
            tool.id
        );
    }
}
