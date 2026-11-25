use super::{Manifest, asset_allowed, check, decode_manifest, signed_manifest, stable_version};
use crate::addons::github_asset;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::Duration,
};
use tauri::Manager;
use tempfile::{NamedTempFile, TempDir};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
pub(super) const FILES: &[&str] = &[
    "tinymist.exe",
    "goodtype-ink.exe",
    "goodtype-mcp.exe",
    "skill/goodtype/SKILL.md",
    "ink-v1.md",
    "README.txt",
    "LICENSE",
    "NOTICE",
    "Goodtype.exe",
];
const MARKER: &str = ".goodtype-update-active";
const HELPER: &str = "Goodtype-Recover.exe";
const MAX_EXPANDED: u64 = 1536 * 1024 * 1024;
#[derive(Default)]
pub(crate) struct Pending(Mutex<Option<(TempDir, bool)>>);

fn regular(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("reparse point".into());
        }
    }
    if m.is_symlink() {
        return Err("symlink".into());
    }
    Ok(())
}

fn file_at(root: &Path, name: &str) -> Result<PathBuf> {
    // Callers pass only names from the compiled package inventory.
    regular(root)?;
    let mut file = root.to_owned();
    for part in name.split('/') {
        file.push(part);
        regular(&file)?;
    }
    if !file.is_file() {
        return Err("expected a package file".into());
    }
    Ok(file)
}

fn hash(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn verified_file(root: &Path, name: &str, manifest: &Manifest) -> Result<PathBuf> {
    let file = file_at(root, name)?;
    if hash(&file)? != manifest.files[name] {
        return Err("package file changed".into());
    }
    Ok(file)
}

fn copy_atomic(source: &Path, target: &Path, expected: &str) -> Result<()> {
    let mut temp = NamedTempFile::new_in(target.parent().ok_or("missing package folder")?)?;
    io::copy(&mut File::open(source)?, temp.as_file_mut())?;
    temp.as_file().sync_all()?;
    if hash(temp.path())? != expected {
        return Err("file changed during copy".into());
    }
    temp.persist(target)?;
    Ok(())
}

fn receipt(dir: &Path) -> Result<Manifest> {
    let bytes = bounded_file(dir, "manifest.json", 65536)?;
    let signature = bounded_file(dir, "manifest.sig", 64)?;
    let version = serde_json::from_slice::<serde_json::Value>(&bytes)?;
    Ok(decode_manifest(
        version["version"].as_str().ok_or("missing version")?,
        &bytes,
        &signature,
    )?)
}

fn bounded_file(root: &Path, name: &str, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(file_at(root, name)?)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("oversized update metadata".into());
    }
    Ok(bytes)
}

fn extract(archive: &Path, dest: &Path, manifest: &Manifest) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    if zip.len() > FILES.len() + 2 {
        return Err("unexpected archive entries".into());
    }
    let mut seen = BTreeSet::new();
    let mut remaining = MAX_EXPANDED;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_owned();
        if entry.is_dir() && ["skill/", "skill/goodtype/"].contains(&name.as_str()) {
            continue;
        }
        if !FILES.contains(&name.as_str()) || !seen.insert(name.clone()) || entry.is_symlink() {
            return Err("unexpected package file".into());
        }
        let size = entry.size();
        remaining = remaining
            .checked_sub(size)
            .ok_or("expanded package too large")?;
        let mut file = File::create_new(dest.join(&name))?;
        let written = io::copy(&mut (&mut entry).take(size + 1), &mut file)?;
        file.sync_all()?;
        if written != size {
            return Err("incorrect file size".into());
        }
        verified_file(dest, &name, manifest)?;
    }
    if seen.len() != FILES.len() {
        return Err("incomplete package".into());
    }
    Ok(())
}

fn stage(installed: &str, version: &str, beta: bool, lang: &str) -> Result<TempDir> {
    if !cfg!(all(windows, target_arch = "x86_64")) || cfg!(debug_assertions) {
        return Err("local".into());
    }
    let exe = std::env::current_exe()?;
    if exe.file_name().and_then(|s| s.to_str()) != Some("Goodtype.exe") {
        return Err("local".into());
    }
    let root = exe.parent().ok_or("missing folder")?;
    if root.join(MARKER).try_exists()? {
        return Err("an update is already active".into());
    }
    let offer = check(installed, beta)?.ok_or("no newer eligible release")?;
    if offer.version != version {
        return Err("release changed".into());
    }
    let dir = tempfile::Builder::new()
        .prefix(".goodtype-update-")
        .tempdir_in(root)?;
    fs::write(dir.path().join("language"), lang)?;
    File::create_new(dir.path().join("control.lock"))?;
    let previous = dir.path().join("previous");
    let next = dir.path().join("next");
    for dest in [&previous, &next] {
        fs::create_dir_all(dest.join("skill/goodtype"))?;
    }
    let old = signed_manifest(installed, Some(&previous))?;
    let new = signed_manifest(version, Some(&next))?;
    for &name in FILES {
        let original = verified_file(root, name, &old)?;
        copy_atomic(&original, &previous.join(name), &old.files[name])?;
    }
    let archive = dir.path().join("download.zip");
    let url = format!("{}/v{version}/{}", super::DOWNLOAD, new.archive);
    let mut response = github_asset(&url, |next, redirects| asset_allowed(&url, next, redirects))
        .map_err(|_| "download failed")?
        .into_reader()
        .take(new.archive_bytes + 1);
    let mut file = File::create_new(&archive)?;
    if io::copy(&mut response, &mut file)? != new.archive_bytes || hash(&archive)? != new.sha256 {
        return Err("download verification failed".into());
    }
    drop(file);
    extract(&archive, &next, &new)?;
    fs::remove_file(archive)?;
    copy_atomic(&exe, &dir.path().join(HELPER), &old.files["Goodtype.exe"])?;
    Ok(dir)
}

#[tauri::command]
pub(crate) async fn stage_app_release(
    app: tauri::AppHandle,
    version: String,
    beta: bool,
) -> std::result::Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.try_lock().map_err(|_| "busy")?;
        let lang = crate::settings::load_app_settings(app.clone())
            .map_err(|_| "settings")?
            .ui_language;
        *slot = Some((
            stage(
                &app.package_info().version.to_string(),
                &version,
                beta,
                &lang,
            )
            .map_err(|e| {
                eprintln!("Update staging failed: {e}");
                "stage"
            })?,
            false,
        ));
        Ok(())
    })
    .await
    .map_err(|_| "stage")?
}

#[tauri::command]
pub(crate) fn arm_app_restart(
    app: tauri::AppHandle,
    armed: bool,
) -> std::result::Result<(), String> {
    let pending = app.state::<Pending>();
    let mut slot = pending.0.try_lock().map_err(|_| "busy")?;
    slot.as_mut().ok_or("missing")?.1 = armed;
    Ok(())
}

pub(crate) fn launch_on_exit(app: &tauri::AppHandle) {
    let pending = app.state::<Pending>();
    let Ok(mut slot) = pending.0.try_lock() else {
        return;
    };
    if let Some((dir, true)) = slot.take() {
        let helper = dir.path().join(HELPER);
        // The staged helper derives its installation from its own path. No arbitrary target arguments.
        let result = (|| -> Result<()> {
            let old = receipt(&dir.path().join("previous"))?;
            if hash(&helper)? != old.files["Goodtype.exe"] {
                return Err("helper changed".into());
            }
            Command::new(helper)
                .arg("--apply-goodtype-update")
                .spawn()?;
            Ok(())
        })();
        if result.is_ok() {
            let _ = dir.keep();
        } else {
            report_failure(Some(dir.path()), &format!("{result:?}"));
        }
    }
}

fn replace(root: &Path, source: &Path, from: &Manifest, to: &Manifest) -> Result<()> {
    // Check all destinations before the first write, then recheck each just before replacement.
    for &name in FILES {
        let current = hash(&file_at(root, name)?)?;
        if current != from.files[name] && current != to.files[name] {
            return Err("modified package".into());
        }
        verified_file(source, name, to)?;
    }
    for &name in FILES {
        let path = file_at(root, name)?;
        if hash(&path)? == to.files[name] {
            continue;
        }
        verified_file(root, name, from)?;
        copy_atomic(&source.join(name), &path, &to.files[name])?;
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_exit(exe: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::Instant;
    let deadline = Instant::now() + Duration::from_secs(15);
    while File::options()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(exe)
        .is_err()
    {
        if Instant::now() >= deadline {
            return Err("another Goodtype instance is in use".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn apply(dir: &Path, update: bool) -> Result<()> {
    let root = dir.parent().ok_or("missing installation")?;
    regular(dir)?;
    let previous = dir.join("previous");
    let next = dir.join("next");
    let old = receipt(&previous)?;
    let new = receipt(&next)?;
    if stable_version(&new.version)? <= stable_version(&old.version)? {
        return Err("invalid update order".into());
    }
    activate(dir, update, root, &old, &new)
}

fn activate(dir: &Path, update: bool, root: &Path, old: &Manifest, new: &Manifest) -> Result<()> {
    let previous = dir.join("previous");
    let next = dir.join("next");
    let exe = file_at(root, "Goodtype.exe")?;
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid staging name")?;
    let marker = root.join(MARKER);
    if update {
        let mut file = NamedTempFile::new_in(root)?;
        file.write_all(name.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&marker)?;
    }
    if bounded_file(root, MARKER, 128)? != name.as_bytes() {
        return Err("different update active".into());
    }
    let control = File::open(file_at(dir, "control.lock")?)?;
    control.try_lock()?;
    #[cfg(windows)]
    wait_for_exit(&exe)?;
    if !update {
        replace(root, &previous, new, old)?;
        fs::remove_file(&marker)?;
        Command::new(exe).spawn()?;
        return Ok(());
    }
    let attempt = (|| -> Result<bool> {
        for &name in FILES {
            verified_file(root, name, old)?;
        }
        replace(root, &next, old, new)?;
        // Child acknowledges completed Tauri setup via its private inherited stdout pipe.
        let mut child = Command::new(&exe)
            .arg("--confirm-goodtype-start")
            .stdout(Stdio::piped())
            .spawn()?;
        let mut output = child.stdout.take().ok_or("missing startup pipe")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut ack = [0; 8];
            let _ = tx.send(output.read_exact(&mut ack).is_ok() && &ack == b"GTREADY\n");
        });
        if rx.recv_timeout(Duration::from_secs(60)) != Ok(true) {
            // A slow, live application may already hold user edits. Leave recovery available.
            if child.try_wait()?.is_none() {
                return Ok(false);
            }
            return Err("updated app failed startup".into());
        }
        Ok(true)
    })();
    if matches!(attempt, Ok(false)) {
        return Ok(());
    }
    if let Err(error) = attempt {
        replace(root, &previous, new, old)?;
        fs::remove_file(&marker)?;
        Command::new(exe).spawn()?;
        return Err(error);
    }
    fs::remove_file(&marker)?;
    Ok(())
}

// Remove only authenticated, updater-owned files. A user-added or changed file is preserved.
fn cleanup(dir: &Path) -> Result<()> {
    let previous = dir.join("previous");
    let next = dir.join("next");
    let old = receipt(&previous)?;
    let new = receipt(&next)?;
    let helper = file_at(dir, HELPER)?;
    if hash(&helper)? != old.files["Goodtype.exe"] {
        return Err("changed helper".into());
    }
    fs::remove_file(helper)?;
    for (folder, manifest) in [(&previous, old), (&next, new)] {
        for &name in FILES {
            fs::remove_file(verified_file(folder, name, &manifest)?)?;
        }
        for name in ["manifest.sig", "manifest.json"] {
            fs::remove_file(file_at(folder, name)?)?;
        }
        for name in ["skill/goodtype", "skill"] {
            fs::remove_dir(folder.join(name))?;
        }
        fs::remove_dir(folder)?;
    }
    if bounded_file(dir, "control.lock", 0)?.is_empty() {
        fs::remove_file(dir.join("control.lock"))?;
    }
    if matches!(bounded_file(dir, "language", 2)?.as_slice(), b"en" | b"de") {
        fs::remove_file(dir.join("language"))?;
    }
    fs::remove_dir(dir)?;
    Ok(())
}

pub(crate) fn confirm_start() {
    if std::env::args_os().nth(1).as_deref() == Some("--confirm-goodtype-start".as_ref()) {
        // Capture the verified recovery path before the parent removes the transaction marker.
        let cleanup_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| recovery_dir(exe.parent()?).ok());
        let _ = std::io::stdout().write_all(b"GTREADY\n");
        let _ = std::io::stdout().flush();
        if let Some(dir) = cleanup_dir {
            std::thread::spawn(move || {
                for _ in 0..60 {
                    std::thread::sleep(Duration::from_secs(1));
                    if dir.parent().is_some_and(|root| !root.join(MARKER).exists())
                        && cleanup(&dir).is_ok()
                    {
                        break;
                    }
                }
            });
        }
    }
}

// An interrupted file replacement always leaves the old or the new executable at its normal
// path. Authenticate the recovery program before handing that executable's lock back to Windows.
fn recovery_dir(root: &Path) -> Result<PathBuf> {
    let name = String::from_utf8(bounded_file(root, MARKER, 128)?)?;
    let suffix = name
        .strip_prefix(".goodtype-update-")
        .ok_or("invalid recovery name")?;
    if suffix.is_empty() || !suffix.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err("invalid recovery name".into());
    }
    let dir = root.join(name);
    regular(&dir)?;
    let previous = receipt(&dir.join("previous"))?;
    if hash(&file_at(&dir, HELPER)?)? != previous.files["Goodtype.exe"] {
        return Err("unverified recovery helper".into());
    }
    Ok(dir)
}

fn recover_on_launch(exe: &Path) -> Result<bool> {
    let root = exe.parent().ok_or("missing root")?;
    if !root.join(MARKER).try_exists()? {
        return Ok(false);
    }
    let dir = recovery_dir(root)?;
    if std::env::args_os().nth(1).as_deref() == Some("--confirm-goodtype-start".as_ref()) {
        let next = receipt(&dir.join("next"))?;
        for &name in FILES {
            verified_file(root, name, &next)?;
        }
        return Ok(false);
    }
    let file = File::open(file_at(&dir, "control.lock")?)?;
    if file.try_lock().is_ok() {
        Command::new(dir.join(HELPER)).spawn()?;
    }
    Ok(true)
}

fn report_failure(dir: Option<&Path>, error: &dyn std::fmt::Display) {
    eprintln!("Goodtype update: {error}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let german = dir
            .and_then(|path| bounded_file(path, "language", 2).ok())
            .is_some_and(|b| b == b"de");
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../../../src/lib/i18n/nativeUpdate.json"))
                .expect("bundled translations");
        let message: Vec<u16> = catalog[if german { "de" } else { "en" }]["recoveryMessage"]
            .as_str()
            .expect("bundled recovery message")
            .encode_utf16()
            .chain([0])
            .collect();
        let title: Vec<u16> = "Goodtype".encode_utf16().chain([0]).collect();
        // Both UTF-16 buffers remain valid until the synchronous dialog returns.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = dir;
}

pub(crate) fn helper_entry() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    if exe.file_name().and_then(|s| s.to_str()) != Some(HELPER) {
        if exe.file_name().and_then(|s| s.to_str()) == Some("Goodtype.exe") {
            match recover_on_launch(&exe) {
                Ok(handled) => return handled,
                Err(error) => {
                    report_failure(None, &error);
                    return true;
                }
            }
        }
        return false;
    }
    if let Some(dir) = exe.parent()
        && dir
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with(".goodtype-update-"))
    {
        let update =
            std::env::args_os().nth(1).as_deref() == Some("--apply-goodtype-update".as_ref());
        if let Err(error) = apply(dir, update) {
            report_failure(Some(dir), &error);
        }
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn package(folder: &Path, tag: &str) -> Manifest {
        fs::create_dir_all(folder.join("skill/goodtype")).unwrap();
        let mut files = std::collections::BTreeMap::new();
        for &name in FILES {
            fs::write(folder.join(name), format!("{tag}-{name}")).unwrap();
            files.insert(name.into(), hash(&folder.join(name)).unwrap());
        }
        Manifest {
            version: tag.into(),
            target: String::new(),
            source_dirty: false,
            source_commit: "a".repeat(40),
            channel: "dev".into(),
            archive: String::new(),
            archive_bytes: 1,
            sha256: String::new(),
            files,
        }
    }

    #[test]
    fn interrupted_replacement_restores_originals_and_preserves_unmanaged_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("portable");
        let old = temp.path().join("previous");
        let next = temp.path().join("next");
        let before = package(&old, "0.3.2");
        let after = package(&next, "0.3.3");
        package(&root, "0.3.2");
        let notebook = root.join("skill/goodtype/user-notes.typ");
        fs::write(&notebook, "my notes").unwrap();
        for &name in &FILES[..4] {
            copy_atomic(&next.join(name), &root.join(name), &after.files[name]).unwrap();
        }
        replace(&root, &old, &after, &before).unwrap();
        for &name in FILES {
            verified_file(&root, name, &before).unwrap();
        }
        replace(&root, &next, &before, &after).unwrap();
        for &name in FILES {
            verified_file(&root, name, &after).unwrap();
        }
        assert_eq!(fs::read(&notebook).unwrap(), b"my notes");
        fs::write(root.join("NOTICE"), "local changes").unwrap();
        assert!(replace(&root, &old, &after, &before).is_err());
        assert_eq!(fs::read(root.join("NOTICE")).unwrap(), b"local changes");
        verified_file(&root, "tinymist.exe", &after).unwrap();
    }

    #[test]
    fn unsigned_extra_archive_entries_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = package(&temp.path().join("fixture"), "0.3.2");
        let archive = temp.path().join("package.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("../extra", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"extra").unwrap();
        zip.finish().unwrap();
        let destination = temp.path().join("out");
        fs::create_dir(&destination).unwrap();
        assert!(extract(&archive, &destination, &manifest).is_err());
        assert!(!temp.path().join("extra").exists());
    }
}

#[cfg(all(test, windows))]
mod windows_smoke {
    use super::*;
    // Run the same activation, exclusive executable check, restart handshake and rollback as the
    // installed helper. The small Windows child lets this run without opening a notebook.
    #[test]
    fn starts_replacement_and_recovers_if_the_next_executable_cannot_start() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Portable Goodtype");
        let stage = root.join(".goodtype-update-smoke");
        let old_dir = stage.join("previous");
        let new_dir = stage.join("next");
        let mut old = super::tests::package(&old_dir, "0.3.1");
        let mut new = super::tests::package(&new_dir, "0.3.2");
        super::tests::package(&root, "0.3.1");
        File::create_new(stage.join("control.lock")).unwrap();
        let source = temp.path().join("fixture.rs");
        fs::write(&source, r#"fn main(){println!("GTREADY");}"#).unwrap();
        assert!(
            Command::new("rustc")
                .arg(&source)
                .arg("-o")
                .arg(old_dir.join("Goodtype.exe"))
                .status()
                .unwrap()
                .success()
        );
        fs::copy(old_dir.join("Goodtype.exe"), root.join("Goodtype.exe")).unwrap();
        fs::copy(old_dir.join("Goodtype.exe"), new_dir.join("Goodtype.exe")).unwrap();
        File::options()
            .append(true)
            .open(new_dir.join("Goodtype.exe"))
            .unwrap()
            .write_all(b"new-build")
            .unwrap();
        old.files.insert(
            "Goodtype.exe".into(),
            hash(&old_dir.join("Goodtype.exe")).unwrap(),
        );
        new.files.insert(
            "Goodtype.exe".into(),
            hash(&new_dir.join("Goodtype.exe")).unwrap(),
        );
        activate(&stage, true, &root, &old, &new).unwrap();
        for &name in FILES {
            verified_file(&root, name, &new).unwrap();
        }
        wait_for_exit(&root.join("Goodtype.exe")).unwrap();
        replace(&root, &old_dir, &new, &old).unwrap();
        fs::write(new_dir.join("Goodtype.exe"), b"invalid test executable").unwrap();
        new.files.insert(
            "Goodtype.exe".into(),
            hash(&new_dir.join("Goodtype.exe")).unwrap(),
        );
        assert!(activate(&stage, true, &root, &old, &new).is_err());
        for &name in FILES {
            verified_file(&root, name, &old).unwrap();
        }
        assert!(!root.join(MARKER).exists());
        wait_for_exit(&root.join("Goodtype.exe")).unwrap();
    }
}
