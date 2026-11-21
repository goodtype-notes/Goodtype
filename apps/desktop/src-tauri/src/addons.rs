//! Host for optional first-party addons. Rust owns release trust, installation and process supervision.
//! The frontend can name catalog entries and invoke their capabilities through fixed commands.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use goodtype_core::{SCHEMA_VERSION, paths};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::store::read_bounded;

mod catalog;
pub(crate) mod import;
pub(crate) mod manager;
mod manifest;
pub(crate) mod recognize;
mod update;

use catalog::CATALOG;
use import::ImportResult;

// ---------------------------------------------------------------------------------------------
// The release Goodtype trusts
// ---------------------------------------------------------------------------------------------

/// One tested importer release, pinned in Goodtype's own source. The checksum published beside
/// the release is not trusted on its own: a compromised release could replace both.
#[derive(Clone, Copy, Debug)]
pub struct PinnedRelease<'a> {
    pub version: &'a str,
    pub url: &'a str,
    pub size: u64,
    pub sha256: &'a str,
    pub decoder_commit: &'a str,
}

/// The fixed facts of one optional addon. Nothing here comes from the frontend or a download.
#[derive(Debug)]
pub struct Addon {
    /// The name the frontend uses, and the install folder's suffix.
    pub id: &'static str,
    /// The name its `--version --json` reports; the executable is this plus `.exe`.
    pub name: &'static str,
    /// `goodtype-<source>-importer`: release ZIPs are `<package>-<version>-windows-x64.zip`.
    pub package: &'static str,
    /// The GitHub repository path, `/owner/name`, exactly as its release URLs spell it.
    pub repository: &'static str,
    /// Verifies signed update metadata. Private halves live only in each repository's publisher.
    pub public_key: Option<[u8; 32]>,
    /// The release this build downloads. `None` means no release is published and tested; the
    /// Library then says the importer is not available yet and nothing is ever requested.
    pub pinned: Option<PinnedRelease<'static>>,
    i18n_prefix: &'static str,
    legacy: Option<&'static str>,
    en: (&'static str, &'static str),
    de: (&'static str, &'static str),
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Import {
        filter: &'static str,
        extensions: &'static [&'static str],
        /// `<batch> 2026-10-22` is the Library folder an import goes into.
        batch: &'static str,
        artwork: bool,
    },
    Recognize {
        recognizes: &'static str,
        check_failure: &'static str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonView {
    id: &'static str,
    i18n_prefix: &'static str,
    #[serde(flatten)]
    capability: CapabilityView,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CapabilityView {
    Import { artwork: bool },
    Recognize { recognizes: &'static str },
}

#[tauri::command]
pub fn addon_catalog() -> Vec<AddonView> {
    CATALOG
        .iter()
        .map(|addon| AddonView {
            id: addon.id,
            i18n_prefix: addon.i18n_prefix,
            capability: match addon.kind {
                Kind::Import { artwork, .. } => CapabilityView::Import { artwork },
                Kind::Recognize { recognizes, .. } => CapabilityView::Recognize { recognizes },
            },
        })
        .collect()
}

impl Addon {
    /// Importers carry a notebook-schema and decoder contract and their own package manifest.
    /// Recognition addons have neither; Goodtype records their file manifest at install.
    fn imports_notebooks(&self) -> bool {
        matches!(self.kind, Kind::Import { .. })
    }

    fn executable(&self) -> String {
        format!("{}.exe", self.name)
    }

    /// The release path every download starts at.
    fn release_path(&self) -> String {
        format!("{}/releases/download/", self.repository)
    }

    fn release_url(&self) -> String {
        format!(
            "https://github.com{}",
            self.release_path().trim_end_matches('/')
        )
    }

    /// The fixed latest-release update descriptor, only ever read by an explicit update check.
    fn latest_descriptor(&self) -> String {
        format!(
            "https://github.com{}/releases/latest/download/importer-update.json",
            self.repository
        )
    }
}

const PROTOCOL_VERSION: u64 = 1;
const PACKAGE_MANIFEST: &str = "importer-manifest.json";
const TARGET: &str = "x86_64-pc-windows-msvc";
/// The only hosts a release download may be redirected to.
const ASSET_HOSTS: [&str; 3] = [
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "github-releases.githubusercontent.com",
];

const MIB: u64 = 1024 * 1024;
/// Absolute ceilings, whatever a pin says.
const MAX_PACKAGE_BYTES: u64 = 512 * MIB;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_UNPACKED_BYTES: u64 = 512 * MIB;
const MAX_PATH_DEPTH: usize = 24;
const MAX_JSON_BYTES: usize = 4 * MIB as usize;
const MAX_REDIRECTS: usize = 5;
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(15 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_TIMEOUT: Duration = Duration::from_secs(180);
/// The importer limits its own decoder to 1.5 GiB; this bounds the converter and decoder together.
const JOB_MEMORY_BYTES: usize = 4 * 1024 * 1024 * 1024;

pub fn platform_supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}

// ---------------------------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------------------------

/// Splits an absolute `https://host/path` URL. Anything else — another scheme, credentials, a port,
/// a relative redirect — is refused rather than interpreted.
fn https_parts(url: &str) -> Option<(String, &str)> {
    let rest = url.strip_prefix("https://")?;
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    if host.is_empty() || host.contains(['@', ':', '\\', '?', '#']) || path.contains('\\') {
        return None;
    }
    Some((host.to_ascii_lowercase(), path))
}

/// The first request must be the tool's own releases on github.com; redirects may also go to
/// GitHub's release-asset hosts, and nowhere else.
pub fn download_url_allowed(tool: &Addon, url: &str, redirected: bool) -> bool {
    let Some((host, path)) = https_parts(url) else {
        return false;
    };
    (host == "github.com" && path.starts_with(&tool.release_path()))
        || (redirected && ASSET_HOSTS.contains(&host.as_str()))
}

fn https_agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|error| error.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("goodtype/", env!("CARGO_PKG_VERSION")))
        .tls_connector(Arc::new(tls))
        .build())
}

fn github_asset(
    tool: &Addon,
    url: &str,
    allow_update_pointer: bool,
) -> Result<ureq::Response, String> {
    let agent = https_agent()?;
    let mut url = url.to_owned();
    for redirects in 0..=MAX_REDIRECTS {
        if !(download_url_allowed(tool, &url, redirects > 0)
            || allow_update_pointer && url == tool.latest_descriptor())
        {
            return Err("refused an unexpected importer release URL".into());
        }
        let response = agent
            .get(&url)
            .call()
            .map_err(|error| format!("cannot read the GitHub release ({error})"))?;
        if !(300..400).contains(&response.status()) {
            return Ok(response);
        }
        url = response
            .header("location")
            .ok_or("GitHub redirected without a location")?
            .to_owned();
    }
    Err("GitHub redirected too many times".into())
}

/// Streams the pinned archive to `into`, hashing as it goes. Returns the SHA-256; the caller
/// compares it with the pin before anything is extracted.
fn download(
    tool: &Addon,
    release: &PinnedRelease,
    into: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<String, String> {
    if release.size == 0 || release.size > MAX_PACKAGE_BYTES {
        return Err("the pinned importer package size is out of range".into());
    }
    let started = Instant::now();
    let response = github_asset(tool, release.url, false)?;
    if let Some(length) = response
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok())
        && length != release.size
    {
        return Err(format!(
            "the package is {length} bytes; this Goodtype version expects {}",
            release.size
        ));
    }
    receive(
        response.into_reader(),
        release.size,
        into,
        cancel,
        started,
        progress,
    )
}

/// Writes exactly `size` bytes from `reader` to `into`, hashing them. Fails on cancellation, the
/// deadline, an interrupted stream, or a body shorter or longer than expected.
fn receive(
    reader: impl Read,
    size: u64,
    into: &Path,
    cancel: &AtomicBool,
    started: Instant,
    progress: &dyn Fn(u64, u64),
) -> Result<String, String> {
    let mut reader = reader.take(size + 1);
    let mut file = fs::File::create(into).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        if started.elapsed() > DOWNLOAD_DEADLINE {
            return Err("the download timed out".into());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("the download was interrupted ({error})"))?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > size {
            return Err("the package is larger than expected".into());
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        progress(received, size);
    }
    if received != size {
        return Err("the download ended early".into());
    }
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const CANCELLED: &str = "cancelled";

// ---------------------------------------------------------------------------------------------
// Archive and package checks
// ---------------------------------------------------------------------------------------------

/// The package's one top-level folder for a version.
fn package_folder(tool: &Addon, version: &str) -> String {
    format!("{}-{version}-windows-x64", tool.package)
}

/// A relative path inside a package or report, as `/`-separated components that are safe on
/// every platform. Returns the components.
fn safe_components(path: &str) -> Result<Vec<&str>, String> {
    if path.is_empty() || path.starts_with('/') || path.contains(['\\', ':', '\0']) {
        return Err(format!("unsafe path `{path}`"));
    }
    let components: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if components.len() > MAX_PATH_DEPTH {
        return Err(format!("path `{path}` is nested too deeply"));
    }
    for component in &components {
        let stem = component.split('.').next().unwrap_or(component);
        if component.is_empty()
            || *component == "."
            || *component == ".."
            || component.ends_with(['.', ' '])
            || component.chars().any(char::is_control)
            || component.contains(['*', '?', '"', '<', '>', '|'])
            || paths::is_windows_reserved(stem)
        {
            return Err(format!("unsafe path `{path}`"));
        }
    }
    Ok(components)
}

/// Unpacks `archive` into `into`, which must not exist yet. Only the expected top folder, no
/// links, no collisions; sizes are counted as bytes are written, not taken from headers.
pub fn extract_package(archive: &Path, top: &str, into: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| format!("not a valid ZIP ({error})"))?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err("the package has too many files".into());
    }
    fs::create_dir(into).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut written = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_symlink() {
            return Err(format!("the package contains a link `{}`", entry.name()));
        }
        let name = entry.name().to_owned();
        let components = safe_components(&name)?;
        if components[0] != top {
            return Err(format!("unexpected entry `{name}` outside {top}/"));
        }
        let inner = &components[1..];
        if inner.is_empty() {
            continue;
        }
        if !seen.insert(inner.join("/").to_lowercase()) {
            return Err(format!("the package lists `{name}` twice"));
        }
        let target = inner
            .iter()
            .fold(into.to_path_buf(), |path, part| path.join(part));
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| format!("cannot write `{name}` ({error})"))?;
        let budget = MAX_UNPACKED_BYTES - written;
        let copied = io::copy(&mut (&mut entry).take(budget + 1), &mut output)
            .map_err(|error| format!("cannot unpack `{name}` ({error})"))?;
        written += copied;
        if written > MAX_UNPACKED_BYTES {
            return Err("the package unpacks to more than 512 MiB".into());
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifest {
    pub version: String,
    pub protocol_version: u64,
    pub notebook_schema_version: u32,
    pub target: String,
    pub decoder_commit: String,
    pub files: Vec<PackageFile>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|error| error.to_string())?;
    Ok(hex(&hasher.finalize()))
}

fn is_package_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// Every file under `dir`, as `/`-separated relative paths. Links are refused outright.
fn package_files(dir: &Path) -> Result<Vec<String>, String> {
    if is_package_link(&fs::symlink_metadata(dir).map_err(|error| error.to_string())?) {
        return Err("the package directory is a link".into());
    }
    let mut files = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        for entry in fs::read_dir(&folder).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "a package file name is not Unicode".to_owned())?;
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            safe_components(&relative)?;
            if is_package_link(&metadata) {
                return Err(format!("the package contains a link `{relative}`"));
            } else if metadata.is_dir() {
                pending.push((entry.path(), relative));
            } else {
                files.push(relative);
            }
        }
    }
    Ok(files)
}

/// Python's own bytecode cache. The bundled interpreter writes `__pycache__/*.pyc` beside the
/// sources it runs; those files are not in the manifest and must not be trusted either (Python can
/// prefer a `.pyc` to its `.py`), so they are deleted before every check and rebuilt from the
/// verified sources.
fn is_bytecode_cache(relative: &str) -> bool {
    relative
        .split('/')
        .any(|part| part.eq_ignore_ascii_case("__pycache__"))
        && relative.to_ascii_lowercase().ends_with(".pyc")
}

/// Checks an unpacked package against its own manifest: every listed file present with the
/// listed size and hash, nothing unlisted apart from Python bytecode caches, which are removed.
/// Returns the manifest.
pub fn verify_package(tool: &Addon, dir: &Path) -> Result<PackageManifest, String> {
    let actual = package_files(dir)?;
    let manifest: PackageManifest =
        serde_json::from_slice(&read_bounded(&dir.join(PACKAGE_MANIFEST), MAX_JSON_BYTES)?)
            .map_err(|error| format!("the package manifest is malformed ({error})"))?;
    let mut listed = HashSet::new();
    for file in &manifest.files {
        safe_components(&file.path)?;
        if !listed.insert(file.path.to_lowercase()) {
            return Err(format!("the manifest lists `{}` twice", file.path));
        }
        let path = file
            .path
            .split('/')
            .fold(dir.to_path_buf(), |path, part| path.join(part));
        let size = fs::metadata(&path)
            .map_err(|_| format!("`{}` is missing from the package", file.path))?
            .len();
        if size != file.size_bytes || !sha256_file(&path)?.eq_ignore_ascii_case(&file.sha256) {
            return Err(format!(
                "`{}` does not match the package manifest",
                file.path
            ));
        }
    }
    for file in actual {
        if file == PACKAGE_MANIFEST || listed.contains(&file.to_lowercase()) {
            continue;
        }
        if is_bytecode_cache(&file) {
            let path = file
                .split('/')
                .fold(dir.to_path_buf(), |path, part| path.join(part));
            fs::remove_file(&path).map_err(|error| format!("cannot clear `{file}` ({error})"))?;
            continue;
        }
        return Err(format!("`{file}` is not listed in the package manifest"));
    }
    if !listed.contains(&tool.executable()) {
        return Err("the package has no importer executable".into());
    }
    Ok(manifest)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolVersion {
    pub name: String,
    pub version: String,
    pub protocol_version: u64,
    #[serde(default)]
    pub notebook_schema_version: u32,
    #[serde(default)]
    pub decoder_commit: String,
}

/// Whether a package (and its own version report) is one this Goodtype can drive. With a pin,
/// the version and decoder must be exactly the pinned ones.
pub fn check_contract(
    expected: &Addon,
    manifest: &PackageManifest,
    tool: Option<&ToolVersion>,
    pin: Option<&PinnedRelease>,
) -> Result<(), String> {
    let schema = if expected.is_legacy_release(&manifest.version, manifest, pin) {
        // These two exact releases predate computational pages. The normal core reader
        // still validates their schema-3 notebooks before publishing them.
        goodtype_core::BINARY_INK_SCHEMA_VERSION
    } else {
        SCHEMA_VERSION
    };
    if manifest.protocol_version != PROTOCOL_VERSION
        || (expected.imports_notebooks() && manifest.notebook_schema_version != schema)
        || manifest.target != TARGET
    {
        return Err(format!(
            "importer {} speaks protocol {} for notebook schema {} on {}; this Goodtype needs \
             protocol {PROTOCOL_VERSION}, schema {schema} on {TARGET}",
            manifest.version,
            manifest.protocol_version,
            manifest.notebook_schema_version,
            manifest.target
        ));
    }
    if let Some(tool) = tool
        && (tool.name != expected.name
            || tool.version != manifest.version
            || tool.protocol_version != manifest.protocol_version
            || (expected.imports_notebooks()
                && tool.notebook_schema_version != manifest.notebook_schema_version)
            || (expected.imports_notebooks() && tool.decoder_commit != manifest.decoder_commit))
    {
        return Err("the importer reports a different version than its package".into());
    }
    if let Some(pin) = pin
        && (manifest.version != pin.version
            || (expected.imports_notebooks() && manifest.decoder_commit != pin.decoder_commit))
    {
        return Err(format!(
            "importer {} is not the tested version {}",
            manifest.version, pin.version
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
struct Active {
    version: String,
}

/// `<app-local-data>/<tool>-importer`: `versions/<version>/`, `active.json`, and transient
/// `staging-*` / `download-*.part` entries that a failed install leaves nothing else of.
pub struct InstallDir(pub PathBuf, pub &'static Addon);

impl InstallDir {
    fn versions(&self) -> PathBuf {
        self.0.join("versions")
    }

    fn version_dir(&self, version: &str) -> Result<PathBuf, String> {
        if !manifest::release_version(version) {
            return Err("invalid importer version".into());
        }
        safe_components(version)?;
        if fs::symlink_metadata(self.versions()).is_ok_and(|metadata| is_package_link(&metadata)) {
            return Err("the addon versions directory is a link".into());
        }
        Ok(self.versions().join(version))
    }

    /// The installed version that `active.json` names, if its folder still holds the tool.
    pub fn active(&self) -> Option<(String, PathBuf)> {
        let bytes = read_bounded(&self.0.join("active.json"), 4096).ok()?;
        let active: Active = serde_json::from_slice(&bytes).ok()?;
        let dir = self.version_dir(&active.version).ok()?;
        dir.join(self.1.executable())
            .is_file()
            .then_some((active.version, dir))
    }

    fn activate(&self, version: &str) -> Result<(), String> {
        let bytes = serde_json::to_vec(&Active {
            version: version.to_owned(),
        })
        .map_err(|error| error.to_string())?;
        crate::store::write_atomic(&self.0.join("active.json"), &bytes)
    }

    fn scratch(&self, prefix: &str) -> PathBuf {
        self.0.join(format!("{prefix}{}", unique_suffix()))
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", std::process::id(), nanos)
}

/// Installs an archive that has already been downloaded to `archive`. Verifies the pinned hash,
/// unpacks, checks the manifest and the tool's own contract, then switches `active.json`. Any
/// failure removes what this attempt created and leaves the previous version active.
#[cfg(test)]
pub fn install_archive(
    dir: &InstallDir,
    archive: &Path,
    release: &PinnedRelease,
    probe: &dyn Fn(&Path) -> Result<ToolVersion, String>,
) -> Result<String, String> {
    install_with_policy(dir, archive, release, probe, |_| {
        dir.activate(release.version)
    })
}

fn install_with_policy(
    dir: &InstallDir,
    archive: &Path,
    release: &PinnedRelease,
    probe: &dyn Fn(&Path) -> Result<ToolVersion, String>,
    activate: impl FnOnce(&manifest::Manifest) -> Result<(), String>,
) -> Result<String, String> {
    if fs::metadata(archive).map_err(|e| e.to_string())?.len() != release.size {
        return Err("the package size differs from the pinned release".into());
    }
    let hash = sha256_file(archive)?;
    if !hash.eq_ignore_ascii_case(release.sha256) {
        return Err("the downloaded package does not match the tested release".into());
    }
    dir.version_dir(release.version)?;
    fs::create_dir_all(dir.versions()).map_err(|error| error.to_string())?;
    let staging = dir.scratch("staging-");
    let mut retired = None;
    let mut installed_target = None;
    let result = (|| {
        extract_package(archive, &package_folder(dir.1, release.version), &staging)?;
        // Recognition addons ship no importer manifest. Record every file from the pinned, verified ZIP locally.
        if !dir.1.imports_notebooks() {
            let files = package_files(&staging)?
                .into_iter()
                .map(|path| {
                    let file = staging.join(&path);
                    Ok(PackageFile {
                        path,
                        size_bytes: fs::metadata(&file).map_err(|e| e.to_string())?.len(),
                        sha256: sha256_file(&file)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let manifest = PackageManifest {
                version: release.version.into(),
                protocol_version: 1,
                notebook_schema_version: 0,
                target: TARGET.into(),
                decoder_commit: String::new(),
                files,
            };
            fs::write(
                staging.join(PACKAGE_MANIFEST),
                serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        let manifest = verify_package(dir.1, &staging)?;
        check_contract(dir.1, &manifest, None, Some(release))?;
        let declaration =
            manifest::load(dir.1, &staging, release.version, &manifest, Some(release))
                .map_err(manager::manifest_problem)?;
        let tool = probe(&staging)?;
        check_contract(dir.1, &manifest, Some(&tool), Some(release))?;
        let target = dir.version_dir(release.version)?;
        if target.exists() {
            // Same version reinstalled: the verified copy replaces the old one.
            let previous = dir.scratch("retired-");
            fs::rename(&target, &previous).map_err(|error| error.to_string())?;
            retired = Some(previous);
        }
        fs::rename(&staging, &target).map_err(|error| error.to_string())?;
        installed_target = Some(target);
        activate(&declaration)?;
        Ok(release.version.to_owned())
    })();
    if result.is_err() {
        if let Some(target) = installed_target {
            let _ = fs::remove_dir_all(target);
        }
        if let Some(previous) = retired {
            fs::rename(previous, dir.version_dir(release.version)?)
                .map_err(|error| format!("could not restore the previous importer ({error})"))?;
        }
    } else {
        if let Some(previous) = retired {
            let _ = fs::remove_dir_all(previous);
        }
        prune_versions(dir, release.version);
    }
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Only installer-owned version directories are cleanup targets, after successful activation.
fn prune_versions(dir: &InstallDir, active: &str) {
    let Ok(root) = fs::canonicalize(dir.versions()) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() == active
            || !entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            continue;
        }
        let Ok(path) = fs::canonicalize(entry.path()) else {
            continue;
        };
        if path.starts_with(&root) && path != root {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// Asks the installed tool for its contract and runs its own decoder check.
fn probe_tool(tool: &Addon, dir: &Path) -> Result<ToolVersion, String> {
    let executable = dir.join(tool.executable());
    let output = run_contained(
        Command::new(&executable)
            .args(["--version", "--json"])
            .current_dir(dir),
        PROBE_TIMEOUT,
        &AtomicBool::new(false),
        &|| {},
    )?;
    if !output.success {
        return Err(format!(
            "the importer did not start ({})",
            output.stderr.trim()
        ));
    }
    let reported: ToolVersion = serde_json::from_str(output.stdout.trim())
        .map_err(|error| format!("the importer's version report is malformed ({error})"))?;
    let check = run_contained(
        Command::new(&executable).arg("--check").current_dir(dir),
        CHECK_TIMEOUT,
        &AtomicBool::new(false),
        &|| {},
    )?;
    if !check.success {
        return Err(format!(
            "the importer's self-check failed ({})",
            check.stderr.trim()
        ));
    }
    if let Kind::Recognize { check_failure, .. } = tool.kind {
        let report: serde_json::Value =
            serde_json::from_str(check.stdout.trim()).map_err(|e| e.to_string())?;
        if report.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            return Err(check_failure.into());
        }
    }
    Ok(reported)
}

// ---------------------------------------------------------------------------------------------
// Running the tool
// ---------------------------------------------------------------------------------------------

struct Finished {
    success: bool,
    cancelled: bool,
    stdout: String,
    stderr: String,
}

/// Keeps the last part of a stream; output is for diagnostics only and never parsed for progress.
fn tail_reader(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        const KEEP: usize = 16 * 1024;
        let mut kept = Vec::new();
        let mut buffer = [0_u8; 4096];
        while let Ok(read) = stream.read(&mut buffer) {
            if read == 0 {
                break;
            }
            kept.extend_from_slice(&buffer[..read]);
            if kept.len() > 2 * KEEP {
                kept.drain(..kept.len() - KEEP);
            }
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

/// Runs the fixed tool without a shell or console window, inside a Job Object that ends the whole
/// process tree when it is cancelled, times out or Goodtype exits. `tick` runs while waiting.
fn run_contained(
    command: &mut Command,
    timeout: Duration,
    cancel: &AtomicBool,
    tick: &dyn Fn(),
) -> Result<Finished, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(windows)]
    let job = job::Job::new(JOB_MEMORY_BYTES).map_err(|error| error.to_string())?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("the importer could not start ({error})"))?;
    #[cfg(windows)]
    if let Err(error) = job.assign(&child) {
        let _ = child.kill();
        return Err(format!("the importer could not be contained ({error})"));
    }
    let stdout = tail_reader(child.stdout.take().expect("piped"));
    let stderr = tail_reader(child.stderr.take().expect("piped"));
    let started = Instant::now();
    let mut cancelled = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if cancel.load(Ordering::SeqCst) || started.elapsed() > timeout {
            cancelled = cancel.load(Ordering::SeqCst);
            #[cfg(windows)]
            job.terminate();
            let _ = child.kill();
            break child.wait().map_err(|error| error.to_string())?;
        }
        tick();
        std::thread::sleep(Duration::from_millis(250));
    };
    let timed_out = !cancelled && !status.success() && started.elapsed() > timeout;
    let mut stderr = stderr.join().unwrap_or_default();
    if timed_out {
        stderr.push_str("\nthe importer ran past its time limit");
    }
    Ok(Finished {
        success: status.success(),
        cancelled,
        stdout: stdout.join().unwrap_or_default(),
        stderr,
    })
}

#[cfg(windows)]
pub(crate) mod job {
    //! A Job Object with kill-on-close and a memory ceiling. Closing the handle (cancel, or this
    //! process exiting) ends every process in it. Lifetime and resource containment only — the
    //! tool still runs with the user's file-system rights.

    use std::{io, os::windows::io::AsRawHandle, process::Child, ptr};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_JOB_MEMORY,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        },
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle is a kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new(memory: usize) -> io::Result<Self> {
            // SAFETY: plain Win32 calls with owned, initialised arguments.
            unsafe {
                let handle = CreateJobObjectW(ptr::null(), ptr::null());
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Self(handle);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.JobMemoryLimit = memory;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(job)
            }
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: the child's handle is valid while `child` is alive.
            if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: valid job handle owned by `self`.
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closing the handle we created; kill-on-close ends anything still running.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// One line of the tool's `import-report.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct ReportEntry {
    pub status: String,
    pub source: String,
    pub target: Option<String>,
    #[serde(default)]
    pub pages: usize,
    pub error: Option<String>,
}

pub fn read_report(path: &Path) -> Result<Vec<ReportEntry>, String> {
    let entries: Vec<ReportEntry> = serde_json::from_slice(&read_bounded(path, MAX_JSON_BYTES)?)
        .map_err(|error| format!("the import report is malformed ({error})"))?;
    if entries.len() > 500 {
        return Err("the import report lists too many notebooks".into());
    }
    Ok(entries)
}

// State and commands
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Activity {
    Idle,
    Checking,
    #[serde(rename_all = "camelCase")]
    Downloading {
        received: u64,
        total: u64,
    },
    Installing,
    #[serde(rename_all = "camelCase")]
    InstallFailed {
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    Importing {
        done: usize,
        total: usize,
        cancelling: bool,
    },
    Publishing,
    #[serde(rename_all = "camelCase")]
    Finished {
        result: ImportResult,
    },
    #[serde(rename_all = "camelCase")]
    ImportFailed {
        message: String,
    },
}

impl Activity {
    fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Downloading { .. }
                | Self::Installing
                | Self::Importing { .. }
                | Self::Publishing
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Default)]
struct State {
    activity: Option<Activity>,
    source: Option<PathBuf>,
    update: Option<update::VerifiedUpdate>,
    update_checked: bool,
    update_problem: Option<String>,
    startup_invalid: bool,
}

/// One importer's job. One download or import at a time per importer.
#[derive(Default)]
pub struct Importer {
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
}

impl Importer {
    fn set(&self, activity: Activity) {
        self.state.lock().unwrap().activity = Some(activity);
    }

    fn busy(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .activity
            .as_ref()
            .is_some_and(Activity::is_busy)
    }

    fn begin(&self, activity: Activity) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.activity.as_ref().is_some_and(Activity::is_busy) {
            return Err("the importer is busy".into());
        }
        self.cancel.store(false, Ordering::SeqCst);
        state.activity = Some(activity);
        Ok(())
    }
}

/// Managed by Tauri: each catalog entry keeps its own job, chosen source and update state.
pub struct Importers(HashMap<&'static str, (Importer, &'static Addon)>);

impl Default for Importers {
    fn default() -> Self {
        Self(
            CATALOG
                .iter()
                .map(|&addon| (addon.id, (Importer::default(), addon)))
                .collect(),
        )
    }
}

impl Importers {
    /// Only ids from the fixed catalog exist.
    fn get(&self, id: &str) -> Result<(&Importer, &'static Addon), String> {
        self.0
            .get(id)
            .map(|(job, addon)| (job, *addon))
            .ok_or_else(|| "unknown importer".into())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseView {
    pub version: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImporterStatus {
    /// Windows x64; elsewhere the importer does not exist.
    pub platform_supported: bool,
    /// The release this build would download, or none while no release is published and tested.
    pub release: Option<ReleaseView>,
    /// Installed version, when its files still match its manifest and contract.
    pub installed: Option<String>,
    pub disabled: bool,
    /// Why an installed copy cannot be used, if it cannot.
    pub installed_problem: Option<String>,
    pub source: Option<SourceView>,
    pub activity: Activity,
    pub update: Option<ReleaseView>,
    pub update_checked: bool,
    pub update_problem: Option<String>,
}

pub(crate) fn install_dir(
    app: &tauri::AppHandle,
    tool: &'static Addon,
) -> Result<InstallDir, String> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?;
    let dir = base.join(format!("{}-importer", tool.id));
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    if is_package_link(&fs::symlink_metadata(&dir).map_err(|error| error.to_string())?) {
        return Err("the addon installation directory is a link".into());
    }
    Ok(InstallDir(dir, tool))
}

/// The installed copy if it is usable: files match its manifest, and its contract fits. Hashes
/// every file. The app runs the same check in the import thread right before a launch.
#[cfg(test)]
fn usable_installation(dir: &InstallDir) -> Result<Option<(String, PathBuf)>, String> {
    let Some((version, path)) = dir.active() else {
        return Ok(None);
    };
    manager::installed(dir.1, &path, &version, true).map_err(manager::manifest_problem)?;
    Ok(Some((version, path)))
}

/// The cheap half of [`usable_installation`] for status: the manifest's contract, no hashing.
pub(crate) fn installed_contract(dir: &InstallDir) -> Result<Option<(String, PathBuf)>, String> {
    let Some((version, path)) = dir.active() else {
        return Ok(None);
    };
    manager::installed(dir.1, &path, &version, false).map_err(manager::manifest_problem)?;
    Ok(Some((version, path)))
}

fn source_view(path: &Path) -> Option<SourceView> {
    Some(SourceView {
        name: path.file_name()?.to_string_lossy().into_owned(),
        size_bytes: fs::metadata(path).ok()?.len(),
    })
}

#[tauri::command]
pub async fn importer_status(
    app: tauri::AppHandle,
    importers: tauri::State<'_, Importers>,
    tool: String,
) -> Result<ImporterStatus, String> {
    let (importer, tool) = importers.get(&tool)?;
    let (activity, source, update, update_checked, update_problem, startup_invalid) = {
        let state = importer.state.lock().unwrap();
        (
            state.activity.clone(),
            state.source.clone(),
            state.update.as_ref().map(|release| release.view()),
            state.update_checked,
            state.update_problem.clone(),
            state.startup_invalid,
        )
    };
    let disabled = platform_supported()
        && install_dir(&app, tool)?.active().is_some()
        && !manager::enabled(&app, tool)?;
    let (installed, installed_problem) = if startup_invalid {
        (None, Some("addon package verification failed".into()))
    } else if platform_supported() && !disabled {
        match installed_contract(&install_dir(&app, tool)?) {
            Ok(found) => (found.map(|(version, _)| version), None),
            Err(problem) => (None, Some(problem)),
        }
    } else {
        (None, None)
    };
    Ok(ImporterStatus {
        platform_supported: platform_supported(),
        release: tool.pinned.map(|pin| ReleaseView {
            version: pin.version.into(),
            size_bytes: pin.size,
        }),
        installed,
        disabled,
        installed_problem,
        source: source.as_deref().and_then(source_view),
        activity: activity.unwrap_or(Activity::Idle),
        update,
        update_checked,
        update_problem,
    })
}

/// Downloads and installs the pinned release. Only ever called from the user's explicit action.
#[tauri::command]
pub fn importer_install(
    app: tauri::AppHandle,
    importers: tauri::State<'_, Importers>,
    tool: String,
) -> Result<(), String> {
    let (importer, tool) = importers.get(&tool)?;
    if !platform_supported() {
        return Err("the importer is available for Windows x64 only".into());
    }
    let Some(release) = tool.pinned else {
        return Err("no tested importer release is available yet".into());
    };
    if importer.busy() {
        return Err("the importer is busy".into());
    }
    let dir = install_dir(&app, tool)?;
    importer.begin(Activity::Downloading {
        received: 0,
        total: release.size,
    })?;
    let state = importer.state.clone();
    let cancel = importer.cancel.clone();
    std::thread::spawn(move || {
        let set = |activity| state.lock().unwrap().activity = Some(activity);
        let archive = dir.scratch("download-").with_extension("part");
        let result = download(tool, &release, &archive, &cancel, &|received, total| {
            set(Activity::Downloading { received, total });
        })
        .and_then(|_| {
            set(Activity::Installing);
            manager::install(&app, &dir, &archive, &release)
        });
        let _ = fs::remove_file(&archive);
        let mut state = state.lock().unwrap();
        state.activity = Some(match result {
            Ok(_) => {
                state.startup_invalid = false;
                Activity::Idle
            }
            Err(message) if message == CANCELLED => Activity::Idle,
            Err(message) => Activity::InstallFailed { message },
        });
    });
    Ok(())
}

#[tauri::command]
pub fn importer_check_update(
    app: tauri::AppHandle,
    importers: tauri::State<'_, Importers>,
    tool: String,
) -> Result<(), String> {
    let (importer, tool) = importers.get(&tool)?;
    if tool.public_key.is_none() {
        return Err("no signed update key is available".into());
    }
    if !platform_supported() || importer.busy() {
        return Err("the importer cannot check for updates right now".into());
    }
    let dir = install_dir(&app, tool)?;
    let installed = dir.active().map(|(version, _)| version);
    importer.begin(Activity::Checking)?;
    let state = importer.state.clone();
    std::thread::spawn(move || {
        let result = update::check(tool, installed.as_deref());
        let mut state = state.lock().unwrap();
        state.update_checked = true;
        state.update = None;
        state.update_problem = None;
        match result {
            Ok(release) => state.update = release,
            Err(message) => state.update_problem = Some(message),
        }
        state.activity = Some(Activity::Idle);
    });
    Ok(())
}

#[tauri::command]
pub fn importer_update(
    app: tauri::AppHandle,
    importers: tauri::State<'_, Importers>,
    tool: String,
) -> Result<(), String> {
    let (importer, tool) = importers.get(&tool)?;
    if tool.public_key.is_none() {
        return Err("no signed update key is available".into());
    }
    if !platform_supported() || importer.busy() {
        return Err("the importer cannot update right now".into());
    }
    let dir = install_dir(&app, tool)?;
    let candidate = importer
        .state
        .lock()
        .unwrap()
        .update
        .clone()
        .ok_or("check for an importer update first")?;
    if let Some((installed, _)) = dir.active() {
        update::require_newer(&candidate.version, &installed)?;
    }
    importer.begin(Activity::Downloading {
        received: 0,
        total: candidate.size,
    })?;
    let state = importer.state.clone();
    let cancel = importer.cancel.clone();
    std::thread::spawn(move || {
        let set = |activity| state.lock().unwrap().activity = Some(activity);
        let archive = dir.scratch("download-").with_extension("part");
        let release = candidate.pin();
        let result = download(tool, &release, &archive, &cancel, &|received, total| {
            set(Activity::Downloading { received, total })
        })
        .and_then(|_| {
            set(Activity::Installing);
            manager::install(&app, &dir, &archive, &release)
        });
        let _ = fs::remove_file(&archive);
        let mut state = state.lock().unwrap();
        state.activity = Some(match result {
            Ok(_) => {
                state.update = None;
                state.startup_invalid = false;
                Activity::Idle
            }
            Err(message) if message == CANCELLED => Activity::Idle,
            Err(message) => Activity::InstallFailed { message },
        });
    });
    Ok(())
}

/// Stops a download or an import. An import keeps every notebook it had finished.
#[tauri::command]
pub fn importer_cancel(importers: tauri::State<'_, Importers>, tool: String) -> Result<(), String> {
    let (importer, _) = importers.get(&tool)?;
    importer.cancel.store(true, Ordering::SeqCst);
    let mut state = importer.state.lock().unwrap();
    if let Some(Activity::Importing { cancelling, .. }) = state.activity.as_mut() {
        *cancelling = true;
    }
    Ok(())
}

/// Clears a finished or failed result so the dialog can start over.
#[tauri::command]
pub fn importer_dismiss(
    importers: tauri::State<'_, Importers>,
    tool: String,
) -> Result<(), String> {
    let (importer, _) = importers.get(&tool)?;
    if !importer.busy() {
        importer.state.lock().unwrap().activity = Some(Activity::Idle);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
