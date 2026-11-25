//! Goodtype portable releases are immutable. Stable promotion changes GitHub metadata only.
use crate::addons::{
    ASSET_HOSTS, https_parts,
    update::{fetch, stable_version, verify_signature},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
pub(crate) mod install;

const LIST: &str = "https://api.github.com/repos/goodtype-notes/Goodtype/releases";
const DOWNLOAD: &str = "https://github.com/goodtype-notes/Goodtype/releases/download";
const KEY: &[u8; 32] = include_bytes!("../release-public-key.bin");

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: String,
    target: String,
    source_dirty: bool,
    source_commit: String,
    channel: String,
    archive: String,
    archive_bytes: u64,
    sha256: String,
    files: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Offer {
    version: String,
    size: u64,
    beta: bool,
}

fn asset_allowed(initial: &str, url: &str, redirected: bool) -> bool {
    url == initial
        || redirected
            && https_parts(url).is_some_and(|(host, _)| ASSET_HOSTS.contains(&host.as_str()))
}

fn signed_manifest(version: &str, receipt: Option<&Path>) -> Result<Manifest, String> {
    stable_version(version)?;
    let base = format!("Goodtype-Windows-{version}");
    let url = format!("{DOWNLOAD}/v{version}/{base}.json");
    let read = |url: &str, limit| {
        fetch(url, limit, |next, redirected| {
            asset_allowed(url, next, redirected)
        })
        .map_err(|_| "unavailable".to_string())
    };
    let bytes = read(&url, 65536)?;
    let signature = read(&format!("{url}.sig"), 64)?;
    let manifest = decode_manifest(version, &bytes, &signature)?;
    if let Some(folder) = receipt {
        std::fs::write(folder.join("manifest.json"), &bytes).map_err(|_| "storage")?;
        std::fs::write(folder.join("manifest.sig"), &signature).map_err(|_| "storage")?;
    }
    Ok(manifest)
}

fn decode_manifest(version: &str, bytes: &[u8], signature: &[u8]) -> Result<Manifest, String> {
    stable_version(version)?;
    let base = format!("Goodtype-Windows-{version}");
    verify_signature(bytes, signature, KEY)?;
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| "invalid")?;
    if manifest.version != version
        || manifest.target != "x86_64-pc-windows-msvc"
        || manifest.channel != "dev"
        || manifest.source_commit.len() != 40
        || !manifest
            .source_commit
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || manifest.source_dirty
        || manifest.archive != format!("{base}.zip")
        || !(1..=512 * 1024 * 1024).contains(&manifest.archive_bytes)
        || manifest.files.len() != install::FILES.len()
        || install::FILES.iter().any(|name| {
            !manifest
                .files
                .get(*name)
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("invalid".into());
    }
    Ok(manifest)
}

fn eligible(release: &Release, installed: [u32; 3], beta: bool) -> Option<[u32; 3]> {
    let version = stable_version(release.tag_name.strip_prefix('v')?).ok()?;
    (!release.draft && (beta || !release.prerelease) && version > installed).then_some(version)
}

fn check(installed: &str, beta: bool) -> Result<Option<Offer>, String> {
    let installed = stable_version(installed).map_err(|_| "local")?;
    let mut best = None;
    for page in 1..=32 {
        let url = format!("{LIST}?per_page=100&page={page}");
        let bytes =
            fetch(&url, 4 * 1024 * 1024, |next, _| next == url).map_err(|_| "unavailable")?;
        let releases: Vec<Release> = serde_json::from_slice(&bytes).map_err(|_| "invalid")?;
        let more = releases.len() == 100;
        for release in releases {
            if let Some(version) = eligible(&release, installed, beta)
                && best
                    .as_ref()
                    .is_none_or(|(previous, _)| version > *previous)
            {
                best = Some((version, release));
            }
        }
        if !more {
            break;
        }
        if page == 32 {
            return Err("unavailable".into());
        }
    }
    let Some((_, release)) = best else {
        return Ok(None);
    };
    let manifest = signed_manifest(&release.tag_name[1..], None)?;
    Ok(Some(Offer {
        version: manifest.version,
        size: manifest.archive_bytes,
        beta: release.prerelease,
    }))
}

#[tauri::command]
pub(crate) async fn check_app_release(
    app: tauri::AppHandle,
    beta: bool,
) -> Result<Option<Offer>, String> {
    if !cfg!(all(windows, target_arch = "x86_64")) || cfg!(debug_assertions) {
        return Err("local".into());
    }
    let version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || check(&version, beta))
        .await
        .map_err(|_| "unavailable")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channels_and_numeric_versions() {
        let mut r = Release {
            tag_name: "v0.3.10".into(),
            draft: false,
            prerelease: true,
        };
        assert_eq!(eligible(&r, [0, 3, 9], true), Some([0, 3, 10]));
        assert_eq!(eligible(&r, [0, 3, 9], false), None);
        r.prerelease = false;
        assert_eq!(eligible(&r, [0, 3, 10], true), None);
        assert_eq!(eligible(&r, [0, 3, 11], false), None);
        r.draft = true;
        assert_eq!(eligible(&r, [0, 3, 9], true), None);
    }
}
