use super::{
    catalog::{GOODNOTES, MATH, ONENOTE},
    import::{batch_name, publish, run_import, today, validate_notebook},
    *,
};
use crate::library;
use goodtype_core::{layout, storage};
use std::io::Cursor;
use zip::write::SimpleFileOptions;

const VERSION: &str = "9.9.9";
/// Every importer must pass every check below; they share one code path.
const TOOLS: [&Addon; 2] = [&GOODNOTES, &ONENOTE];

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        if name.ends_with('/') {
            writer
                .add_directory(*name, SimpleFileOptions::default())
                .unwrap();
        } else {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A small but complete package: the executable stand-in, a runtime file, and a manifest
/// listing both with their hashes.
fn package(tool: &Addon, version: &str, tamper: bool, extra: Option<(&str, &[u8])>) -> Vec<u8> {
    let top = package_folder(tool, version);
    let executable = tool.executable();
    let exe: &[u8] = b"not really an executable";
    let runtime: &[u8] = b"print('decoder')";
    let manifest = serde_json::json!({
        "version": version, "protocolVersion": 1, "notebookSchemaVersion": SCHEMA_VERSION,
        "target": TARGET, "decoderCommit": "abc",
        "files": [
            {"path": executable, "sizeBytes": exe.len(), "sha256": sha(exe)},
            {"path": "runtime/decode.py", "sizeBytes": runtime.len(), "sha256": if tamper { "00".repeat(32) } else { sha(runtime) }},
        ]
    })
    .to_string();
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (format!("{top}/{executable}"), exe.to_vec()),
        (format!("{top}/runtime/decode.py"), runtime.to_vec()),
        (format!("{top}/{PACKAGE_MANIFEST}"), manifest.into_bytes()),
    ];
    if let Some((name, bytes)) = extra {
        entries.push((format!("{top}/{name}"), bytes.to_vec()));
    }
    let borrowed: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    zip_bytes(&borrowed)
}

fn pin(tool: &Addon, bytes: &[u8]) -> PinnedRelease<'static> {
    PinnedRelease {
        version: VERSION,
        url: Box::leak(format!("{}/v9.9.9/x.zip", tool.release_url()).into_boxed_str()),
        size: bytes.len() as u64,
        sha256: Box::leak(sha(bytes).into_boxed_str()),
        decoder_commit: "abc",
    }
}

fn good_probe(tool: &Addon) -> Result<ToolVersion, String> {
    Ok(ToolVersion {
        name: tool.name.into(),
        version: VERSION.into(),
        protocol_version: 1,
        notebook_schema_version: SCHEMA_VERSION,
        decoder_commit: "abc".into(),
    })
}

#[test]
fn downloads_only_from_the_tools_own_releases_and_github_asset_hosts() {
    for tool in TOOLS {
        let release = format!("{}/v0.1.0/a.zip", tool.release_url());
        let repository = tool.repository;
        assert!(download_url_allowed(tool, &release, false));
        assert!(download_url_allowed(
            tool,
            "https://objects.githubusercontent.com/x?sig=1",
            true
        ));
        for refused in [
            "https://objects.githubusercontent.com/x".to_owned(),
            release.replace("https", "http"),
            "https://github.com/someone/else/releases/download/a.zip".to_owned(),
            format!("https://github.com@evil.example{repository}/releases/download/a"),
            format!("https://github.com:8443{repository}/releases/download/a"),
            // The repository's other pages are not release downloads.
            format!("https://github.com{repository}/archive/main.zip"),
        ] {
            assert!(!download_url_allowed(tool, &refused, false), "{refused}");
        }
        assert!(!download_url_allowed(tool, "https://evil.example/x", true));
        assert!(!download_url_allowed(tool, "/relative/redirect", true));
    }
    // One importer's releases are never a permitted download for the other.
    for (tool, other) in [(&GOODNOTES, &ONENOTE), (&ONENOTE, &GOODNOTES)] {
        let release = format!("{}/v0.1.0/a.zip", other.release_url());
        assert!(!download_url_allowed(tool, &release, false));
        assert!(!download_url_allowed(tool, &release, true));
    }
}

/// A stream that delivers some bytes and then fails, like a dropped connection.
struct Interrupted(usize);

impl Read for Interrupted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "connection reset",
            ));
        }
        let count = self.0.min(buffer.len());
        buffer[..count].fill(7);
        self.0 -= count;
        Ok(count)
    }
}

#[test]
fn interrupted_short_long_and_cancelled_downloads_fail() {
    let home = tempfile::tempdir().unwrap();
    let into = home.path().join("download.part");
    let quiet = |_: u64, _: u64| {};
    let go = AtomicBool::new(false);
    let now = Instant::now();
    let bytes = vec![1_u8; 1000];
    let expected = sha(&bytes);
    assert_eq!(
        receive(bytes.as_slice(), 1000, &into, &go, now, &quiet).unwrap(),
        expected
    );
    assert!(
        receive(Interrupted(500), 1000, &into, &go, now, &quiet)
            .unwrap_err()
            .contains("interrupted")
    );
    assert!(
        receive(&bytes[..999], 1000, &into, &go, now, &quiet)
            .unwrap_err()
            .contains("early")
    );
    assert!(
        receive(vec![1_u8; 1001].as_slice(), 1000, &into, &go, now, &quiet)
            .unwrap_err()
            .contains("larger")
    );
    let stop = AtomicBool::new(true);
    assert_eq!(
        receive(bytes.as_slice(), 1000, &into, &stop, now, &quiet).unwrap_err(),
        CANCELLED
    );
    let late = Instant::now() - DOWNLOAD_DEADLINE - Duration::from_secs(1);
    assert!(
        receive(bytes.as_slice(), 1000, &into, &go, late, &quiet)
            .unwrap_err()
            .contains("timed out")
    );
}

#[test]
fn installs_a_verified_package_and_keeps_it_when_a_later_one_is_bad() {
    for tool in TOOLS {
        let home = tempfile::tempdir().unwrap();
        let dir = InstallDir(home.path().to_path_buf(), tool);
        let good = package(tool, VERSION, false, None);
        let archive = home.path().join("good.zip");
        fs::write(&archive, &good).unwrap();
        let probe = |_: &Path| good_probe(tool);
        assert_eq!(
            install_archive(&dir, &archive, &pin(tool, &good), &probe).unwrap(),
            VERSION
        );
        assert_eq!(dir.active().unwrap().0, VERSION);
        assert!(usable_installation(&dir).unwrap().is_some());

        // Wrong checksum, tampered file, unlisted file, failing probe, the other importer's
        // package or version report: each refused, the installed version untouched and no
        // staging left behind.
        let other = if tool.id == "goodnotes" {
            &ONENOTE
        } else {
            &GOODNOTES
        };
        let tampered = package(tool, VERSION, true, None);
        let unlisted = package(
            tool,
            VERSION,
            false,
            Some(("runtime/extra.dll", b"surprise")),
        );
        let foreign = package(other, VERSION, false, None);
        let foreign_probe = |_: &Path| good_probe(other);
        type Probe<'a> = &'a dyn Fn(&Path) -> Result<ToolVersion, String>;
        let cases: Vec<(Vec<u8>, PinnedRelease, Probe)> = vec![
            (
                good.clone(),
                PinnedRelease {
                    sha256: "00",
                    ..pin(tool, &good)
                },
                &probe,
            ),
            (tampered.clone(), pin(tool, &tampered), &probe),
            (unlisted.clone(), pin(tool, &unlisted), &probe),
            (foreign.clone(), pin(tool, &foreign), &probe),
            (good.clone(), pin(tool, &good), &foreign_probe),
            (good.clone(), pin(tool, &good), &|_| {
                Err("self-check failed".into())
            }),
        ];
        for (bytes, release, probe) in cases {
            fs::write(&archive, &bytes).unwrap();
            assert!(install_archive(&dir, &archive, &release, probe).is_err());
            assert_eq!(dir.active().unwrap().0, VERSION);
            assert!(usable_installation(&dir).unwrap().is_some());
            let leftovers = fs::read_dir(home.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("staging-"))
                .count();
            assert_eq!(leftovers, 0);
        }
    }
}

#[test]
fn activation_failure_restores_same_version_and_success_cleans_inactive_versions() {
    let home = tempfile::tempdir().unwrap();
    let dir = InstallDir(home.path().to_path_buf(), &GOODNOTES);
    let good = package(&GOODNOTES, VERSION, false, None);
    let archive = home.path().join("good.zip");
    fs::write(&archive, &good).unwrap();
    install_archive(&dir, &archive, &pin(&GOODNOTES, &good), &|_| {
        good_probe(&GOODNOTES)
    })
    .unwrap();
    let installed = dir.active().unwrap().1;
    // Refuse replacement while keeping the previous pointer readable.
    let active = home.path().join("active.json");
    #[cfg(windows)]
    let active_lock = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&active)
            .unwrap()
    };
    #[cfg(unix)]
    {
        fs::rename(&active, home.path().join("active-before.json")).unwrap();
        std::os::unix::fs::symlink(home.path().join("active-before.json"), &active).unwrap();
    }
    assert!(
        install_archive(&dir, &archive, &pin(&GOODNOTES, &good), &|_| good_probe(
            &GOODNOTES
        ))
        .is_err()
    );
    assert_eq!(dir.active().unwrap().1, installed);
    assert!(usable_installation(&dir).unwrap().is_some());
    #[cfg(windows)]
    drop(active_lock);
    #[cfg(unix)]
    {
        fs::remove_file(&active).unwrap();
        fs::rename(home.path().join("active-before.json"), &active).unwrap();
    }
    let old = dir.version_dir("0.1.0").unwrap();
    fs::create_dir(&old).unwrap();
    fs::write(old.join("old.exe"), b"old importer only").unwrap();
    let notebook = home.path().join("notebooks");
    fs::create_dir(&notebook).unwrap();
    fs::write(notebook.join("goodtype.json"), b"preserve").unwrap();
    install_archive(&dir, &archive, &pin(&GOODNOTES, &good), &|_| {
        good_probe(&GOODNOTES)
    })
    .unwrap();
    assert!(!old.exists());
    assert!(installed.exists());
    assert!(notebook.join("goodtype.json").is_file());
}

#[test]
fn python_caches_are_cleared_but_other_additions_still_fail() {
    let home = tempfile::tempdir().unwrap();
    let dir = InstallDir(home.path().to_path_buf(), &GOODNOTES);
    let good = package(&GOODNOTES, VERSION, false, None);
    let archive = home.path().join("good.zip");
    fs::write(&archive, &good).unwrap();
    install_archive(&dir, &archive, &pin(&GOODNOTES, &good), &|_| {
        good_probe(&GOODNOTES)
    })
    .unwrap();
    let installed = dir.active().unwrap().1;
    let cache = installed.join("runtime/__pycache__/decode.cpython-314.pyc");
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, b"compiled, possibly tampered").unwrap();
    assert!(verify_package(&GOODNOTES, &installed).is_ok());
    assert!(!cache.exists(), "the cache must be removed, not trusted");
    fs::write(
        installed.join("runtime/__pycache__/sneaky.py"),
        b"import os",
    )
    .unwrap();
    assert!(verify_package(&GOODNOTES, &installed).is_err());
}

#[test]
fn refuses_unsafe_archives() {
    for tool in TOOLS {
        let top = package_folder(tool, VERSION);
        let other = package_folder(
            if tool.id == "goodnotes" {
                &ONENOTE
            } else {
                &GOODNOTES
            },
            VERSION,
        );
        let home = tempfile::tempdir().unwrap();
        let cases: Vec<Vec<(String, &[u8])>> = vec![
            vec![(format!("{top}/../escape.txt"), b"x")],
            vec![("C:/Windows/evil.dll".into(), b"x")],
            vec![("/abs.txt".into(), b"x")],
            vec![(format!("{top}\\back\\slash.txt"), b"x")],
            vec![("other-folder/a.txt".into(), b"x")],
            vec![(format!("{other}/a.txt"), b"x")],
            vec![(format!("{top}/CON.txt"), b"x")],
            vec![
                (format!("{top}/a.txt"), b"x"),
                (format!("{top}/A.TXT"), b"y"),
            ],
        ];
        for (index, entries) in cases.iter().enumerate() {
            let borrowed: Vec<(&str, &[u8])> =
                entries.iter().map(|(n, b)| (n.as_str(), *b)).collect();
            let archive = home.path().join(format!("{index}.zip"));
            fs::write(&archive, zip_bytes(&borrowed)).unwrap();
            let into = home.path().join(format!("out-{index}"));
            assert!(
                extract_package(&archive, &top, &into).is_err(),
                "{} case {index} was accepted",
                tool.id
            );
        }
    }
}

#[test]
fn refuses_links_and_decompression_beyond_the_budget() {
    let top = package_folder(&GOODNOTES, VERSION);
    let home = tempfile::tempdir().unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink(
            format!("{top}/link"),
            "../../outside",
            SimpleFileOptions::default(),
        )
        .unwrap();
    let archive = home.path().join("link.zip");
    fs::write(&archive, writer.finish().unwrap().into_inner()).unwrap();
    assert!(extract_package(&archive, &top, &home.path().join("a")).is_err());

    // 513 MiB of zeros compresses to well under a megabyte.
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            format!("{top}/bomb.bin"),
            SimpleFileOptions::default().large_file(true),
        )
        .unwrap();
    let chunk = vec![0_u8; MIB as usize];
    for _ in 0..513 {
        writer.write_all(&chunk).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert!((bytes.len() as u64) < 4 * MIB);
    let archive = home.path().join("bomb.zip");
    fs::write(&archive, bytes).unwrap();
    let error = extract_package(&archive, &top, &home.path().join("b")).unwrap_err();
    assert!(error.contains("512 MiB"), "{error}");
}

#[test]
fn contract_needs_matching_protocol_schema_and_pin() {
    let manifest = |protocol, schema| PackageManifest {
        version: VERSION.into(),
        protocol_version: protocol,
        notebook_schema_version: schema,
        target: TARGET.into(),
        decoder_commit: "abc".into(),
        files: Vec::new(),
    };
    for tool in TOOLS {
        let release = pin(tool, b"x");
        let check =
            |manifest: &PackageManifest, version, pin| check_contract(tool, manifest, version, pin);
        let reported = good_probe(tool).unwrap();
        assert!(
            check(
                &manifest(1, SCHEMA_VERSION),
                Some(&reported),
                Some(&release)
            )
            .is_ok()
        );
        assert!(check(&manifest(2, SCHEMA_VERSION), None, None).is_err());
        assert!(check(&manifest(1, SCHEMA_VERSION + 1), None, None).is_err());
        let other_release = PinnedRelease {
            version: "1.0.0",
            ..release
        };
        assert!(check(&manifest(1, SCHEMA_VERSION), None, Some(&other_release)).is_err());
        let mut lying = good_probe(tool).unwrap();
        lying.version = "0.0.1".into();
        assert!(check(&manifest(1, SCHEMA_VERSION), Some(&lying), None).is_err());
        // A package whose executable reports the other importer's name is not this one.
        let other = if tool.id == "goodnotes" {
            &ONENOTE
        } else {
            &GOODNOTES
        };
        let impostor = good_probe(other).unwrap();
        assert!(check(&manifest(1, SCHEMA_VERSION), Some(&impostor), None).is_err());
    }
}

fn fixture_notebook(into: &Path) {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/notebooks/multipage");
    copy_dir(&fixture, into);
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn entry(status: &str, source: &str, target: Option<&str>, error: Option<&str>) -> ReportEntry {
    ReportEntry {
        status: status.into(),
        source: source.into(),
        target: target.map(Into::into),
        pages: 2,
        error: error.map(Into::into),
    }
}

#[test]
fn publishes_only_validated_completed_notebooks_without_replacing_anything() {
    let home = tempfile::tempdir().unwrap();
    let tool_batch = home.path().join("staging/batch");
    fixture_notebook(&tool_batch.join("Mechanics/Lecture"));
    fixture_notebook(&tool_batch.join("Existing"));
    // Completed but broken (manifest without pages), and a cancelled half-written folder.
    fs::create_dir_all(tool_batch.join("Broken")).unwrap();
    fs::write(tool_batch.join("Broken/goodtype.json"), "{}").unwrap();
    fs::create_dir_all(tool_batch.join("Half")).unwrap();
    let library_batch = home.path().join("Library/Goodnotes-Import");
    fixture_notebook(&library_batch.join("Existing"));
    let before = fs::read(library_batch.join("Existing/goodtype.json")).unwrap();

    let entries = vec![
        entry(
            "completed",
            "Mechanics/Lecture",
            Some("Mechanics/Lecture"),
            None,
        ),
        entry("completed", "Existing", Some("Existing"), None),
        entry("completed", "Broken", Some("Broken"), None),
        entry("completed", "Escape", Some("../../outside"), None),
        entry(
            "failed",
            "Sketches",
            None,
            Some("page 3 needs vector artwork — use --allow-derived"),
        ),
        entry("failed", "Audio", None, Some("unsupported audio")),
        entry("pending", "Half", Some("Half"), None),
    ];
    let outcomes = publish(
        &entries,
        &tool_batch,
        (&library_batch, "Goodnotes-Import"),
        true,
    );
    let statuses: Vec<_> = outcomes.iter().map(|outcome| outcome.status).collect();
    assert_eq!(
        statuses,
        [
            "imported",
            "alreadyThere",
            "failed",
            "failed",
            "needsArtwork",
            "failed",
            "cancelled"
        ]
    );
    assert_eq!(
        outcomes[0].path.as_deref(),
        Some("Goodnotes-Import/Mechanics/Lecture")
    );
    assert!(validate_notebook(&library_batch.join("Mechanics/Lecture")).is_ok());
    assert_eq!(
        fs::read(library_batch.join("Existing/goodtype.json")).unwrap(),
        before
    );
    assert!(!library_batch.join("Broken").exists());
    assert!(!library_batch.join("Half").exists());
    assert!(!home.path().join("outside").exists());
}

#[test]
fn a_pending_transaction_is_refused_rather_than_recovered() {
    let home = tempfile::tempdir().unwrap();
    let notebook = home.path().join("n");
    fixture_notebook(&notebook);
    fs::create_dir_all(notebook.join(".goodtype")).unwrap();
    fs::write(notebook.join(layout::PENDING_TRANSACTION), b"partial").unwrap();
    assert!(validate_notebook(&notebook).is_err());
    assert_eq!(
        fs::read(notebook.join(layout::PENDING_TRANSACTION)).unwrap(),
        b"partial"
    );
}

#[test]
fn malformed_or_oversized_reports_are_refused() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("import-report.json");
    fs::write(&path, b"{not json").unwrap();
    assert!(read_report(&path).is_err());
    fs::write(&path, vec![b' '; MAX_JSON_BYTES + 1]).unwrap();
    assert!(read_report(&path).is_err());
    fs::write(
        &path,
        br#"[{"status":"completed","source":"a","target":"a","pages":1,"error":null}]"#,
    )
    .unwrap();
    assert_eq!(read_report(&path).unwrap().len(), 1);
}

#[test]
fn math_archive_uses_shared_install_and_detects_tampering() {
    let home = tempfile::tempdir().unwrap();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            format!("{}/{}", package_folder(&MATH, VERSION), MATH.executable()),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"test helper").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let archive = home.path().join("math.zip");
    fs::write(&archive, &bytes).unwrap();
    let release = pin(&MATH, &bytes);
    let dir = InstallDir(home.path().join("installed"), &MATH);
    fs::create_dir_all(&dir.0).unwrap();
    install_archive(&dir, &archive, &release, &|_| good_probe(&MATH)).unwrap();
    let (_, installed) = usable_installation(&dir).unwrap().unwrap();
    fs::write(installed.join(MATH.executable()), b"tampered").unwrap();
    assert!(usable_installation(&dir).is_err());
    // A failed replacement does not erase the installed version.
    assert!(install_archive(&dir, &archive, &release, &|_| Err("probe failed".into())).is_err());
    assert!(dir.active().is_some());
}

#[test]
fn math_manifest_requires_name_version_and_protocol() {
    let manifest = PackageManifest {
        version: "0.1.0".into(),
        protocol_version: 1,
        notebook_schema_version: 0,
        target: TARGET.into(),
        decoder_commit: String::new(),
        files: vec![],
    };
    let mut report: ToolVersion = serde_json::from_str(r#"{"name":"goodtype-math-recognize","version":"0.1.0","protocolVersion":1,"model":{"id":"texteller-int8"}}"#).unwrap();
    assert!(check_contract(&MATH, &manifest, Some(&report), None).is_ok());
    report.protocol_version = 2;
    assert!(check_contract(&MATH, &manifest, Some(&report), None).is_err());
    report.protocol_version = 1;
    report.name = "other".into();
    assert!(check_contract(&MATH, &manifest, Some(&report), None).is_err());
    assert!(MATH.pinned.is_none());
    assert!(MATH.public_key.is_none());
    assert!(!download_url_allowed(
        &MATH,
        "https://example.com/math.zip",
        false
    ));
}

#[test]
#[ignore = "needs MATH_RECOGNIZER_RELEASE_ZIP and MATH_RECOGNIZER_RELEASE_SHA256"]
fn real_math_release_installs_and_checks() {
    let archive = PathBuf::from(
        std::env::var("MATH_RECOGNIZER_RELEASE_ZIP").expect("set MATH_RECOGNIZER_RELEASE_ZIP"),
    );
    let sha256 = std::env::var("MATH_RECOGNIZER_RELEASE_SHA256")
        .expect("set MATH_RECOGNIZER_RELEASE_SHA256");
    let release = PinnedRelease {
        version: "0.1.0",
        url: "",
        size: fs::metadata(&archive).unwrap().len(),
        sha256: &sha256,
        decoder_commit: "",
    };
    let home = tempfile::tempdir().unwrap();
    // Set MATH_RECOGNIZER_INSTALL_INTO to the app's own `math-importer` folder to try an
    // unpublished build in the app; the same verified install path is used either way.
    let target = std::env::var("MATH_RECOGNIZER_INSTALL_INTO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.path().join("installed"));
    let dir = InstallDir(target, &MATH);
    fs::create_dir_all(&dir.0).unwrap();
    install_archive(&dir, &archive, &release, &|path| probe_tool(&MATH, path)).unwrap();
    assert_eq!(usable_installation(&dir).unwrap().unwrap().0, "0.1.0");
}

/// Opt-in check against a real release package and real Goodnotes exports, which never belong
/// in the repository:
///
/// ```text
/// GOODNOTES_RELEASE_ZIP=…zip GOODNOTES_RELEASE_SHA256=… GOODNOTES_SMALL=….goodnotes \
/// GOODNOTES_BACKUP=….zip GOODNOTES_LARGE=….goodnotes cargo test real_release -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a downloaded release package and private Goodnotes exports"]
fn real_release_installs_imports_and_cancels() {
    let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
    let archive = PathBuf::from(env("GOODNOTES_RELEASE_ZIP"));
    let release = PinnedRelease {
        version: "0.1.0",
        url: "https://github.com/Just8it/Goodtype-Goodnodes-importer/releases/download/v0.1.0/goodtype-goodnotes-importer-0.1.0-windows-x64.zip",
        size: fs::metadata(&archive).unwrap().len(),
        sha256: Box::leak(env("GOODNOTES_RELEASE_SHA256").into_boxed_str()),
        decoder_commit: "66028455fd7fc61b7931679915d811a25485e3ad",
    };
    let home = tempfile::tempdir().unwrap();
    let dir = InstallDir(home.path().join("installed"), &GOODNOTES);
    fs::create_dir_all(&dir.0).unwrap();
    let started = Instant::now();
    assert_eq!(
        install_archive(&dir, &archive, &release, &|path| probe_tool(
            &GOODNOTES, path
        ))
        .unwrap(),
        "0.1.0"
    );
    println!("installed and self-checked in {:?}", started.elapsed());
    let (_, tool) = usable_installation(&dir).unwrap().unwrap();

    let library = home.path().join("Library");
    fs::create_dir_all(&library).unwrap();
    let state = Mutex::new(State::default());
    let run = |source: &str, batch: &str, derived: bool, cancel: &AtomicBool| {
        let staging = library.join(format!(".goodtype-import-{batch}-{derived}"));
        fs::create_dir(&staging).unwrap();
        let folder = library.join(batch);
        let started = Instant::now();
        let result = run_import(
            &GOODNOTES,
            &tool,
            Path::new(source),
            &staging,
            (&folder, batch),
            derived,
            cancel,
            &state,
        );
        let _ = fs::remove_dir_all(&staging);
        println!(
            "{batch} (derived={derived}) in {:?}: {result:#?}",
            started.elapsed()
        );
        result.unwrap()
    };
    let go = AtomicBool::new(false);

    // Without vector artwork the small notebook needs it; with it, the retry fills the gap.
    let first = run(&env("GOODNOTES_SMALL"), "Small", false, &go);
    assert_eq!(first.notebooks[0].status, "needsArtwork");
    assert!(
        first.batch.is_none(),
        "an empty batch folder must not be left behind"
    );
    let second = run(&env("GOODNOTES_SMALL"), "Small", true, &go);
    assert_eq!(second.notebooks[0].status, "imported");
    assert!(second.notebooks[0].artwork_pages > 0);

    let backup = run(&env("GOODNOTES_BACKUP"), "Backup", true, &go);
    assert!(
        backup
            .notebooks
            .iter()
            .all(|notebook| notebook.status == "imported")
    );
    for notebook in &backup.notebooks {
        let path = library.join(notebook.path.as_ref().unwrap());
        validate_notebook(&path).unwrap();
        assert!(path.join("originals/source.goodnotes").is_file());
    }

    // Cancelling the large notebook mid-conversion publishes nothing half-written.
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        trigger.store(true, Ordering::SeqCst);
    });
    let cancelled = run(&env("GOODNOTES_LARGE"), "Large", true, &cancel);
    timer.join().unwrap();
    assert!(cancelled.cancelled);
    assert!(
        cancelled
            .notebooks
            .iter()
            .all(|notebook| notebook.status != "imported")
    );
    assert!(!library.join("Large").exists());
    let leftovers: Vec<_> = fs::read_dir(&library)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    println!("library now holds {leftovers:?}");
    assert!(
        leftovers
            .iter()
            .all(|name| !name.starts_with(".goodtype-import"))
    );

    // Without the importer installed, an imported notebook edits, saves, moves and reopens.
    fs::remove_dir_all(&dir.0).unwrap();
    let notebook = library.join(backup.notebooks[0].path.as_ref().unwrap());
    let mut snapshot = storage::open_notebook(&notebook).unwrap();
    snapshot.manifest.title = "Edited after import".into();
    storage::save_notebook(&notebook, &snapshot).unwrap();
    let moved = library.join("Moved notebook");
    fs::rename(&notebook, &moved).unwrap();
    validate_notebook(&moved).unwrap();
    assert_eq!(
        storage::open_notebook(&moved).unwrap().manifest.title,
        "Edited after import"
    );
    println!("edited, saved, moved and reopened without the importer installed");
}

/// Opt-in: the pinned OneNote release, installed and run exactly as the app does it, on a
/// OneNote export that never belongs in the repository:
///
/// ```text
/// ONENOTE_RELEASE_ZIP=…zip ONENOTE_EXPORT=….onepkg cargo test real_onenote -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs the downloaded OneNote release package and a OneNote export"]
fn real_onenote_release_installs_and_imports() {
    let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
    let archive = PathBuf::from(env("ONENOTE_RELEASE_ZIP"));
    let release = ONENOTE.pinned.expect("a pinned OneNote release");
    let home = tempfile::tempdir().unwrap();
    let dir = InstallDir(home.path().join("installed"), &ONENOTE);
    fs::create_dir_all(&dir.0).unwrap();
    install_archive(&dir, &archive, &release, &|path| probe_tool(&ONENOTE, path)).unwrap();
    let (_, installed) = usable_installation(&dir).unwrap().unwrap();

    let library = home.path().join("Library");
    let staging = library.join(".goodtype-import-test");
    fs::create_dir_all(&staging).unwrap();
    let name = batch_name(&ONENOTE, &library, &today()).unwrap();
    let folder = library.join(&name);
    let state = Mutex::new(State::default());
    let result = run_import(
        &ONENOTE,
        &installed,
        Path::new(&env("ONENOTE_EXPORT")),
        &staging,
        (&folder, &name),
        false,
        &AtomicBool::new(false),
        &state,
    )
    .unwrap();
    fs::remove_dir_all(&staging).unwrap();
    println!("{result:#?}");
    assert!(result.problem.is_none());
    assert!(!result.notebooks.is_empty());
    for notebook in &result.notebooks {
        assert_eq!(notebook.status, "imported", "{notebook:?}");
        let path = library.join(notebook.path.as_ref().unwrap());
        validate_notebook(&path).unwrap();
        assert!(path.join("originals/source.one").is_file());
    }
}

#[test]
fn the_pinned_releases_are_permitted_downloads() {
    for tool in TOOLS {
        let pin = tool.pinned.expect("a tested release is pinned");
        assert!(download_url_allowed(tool, pin.url, false), "{}", tool.id);
        assert!(pin.size > 0 && pin.size <= MAX_PACKAGE_BYTES);
        assert_eq!(pin.sha256.len(), 64);
        assert_eq!(pin.decoder_commit.len(), 40);
        assert!(
            pin.url
                .ends_with(&format!("{}.zip", package_folder(tool, pin.version)))
        );
    }
    assert_ne!(GOODNOTES.id, ONENOTE.id);
    assert_ne!(GOODNOTES.repository, ONENOTE.repository);
}

#[test]
fn batch_folders_never_reuse_a_name() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        batch_name(&GOODNOTES, home.path(), "2026-10-21").unwrap(),
        "Goodnotes Import 2026-10-21"
    );
    fs::create_dir(home.path().join("Goodnotes Import 2026-10-21")).unwrap();
    assert_eq!(
        batch_name(&GOODNOTES, home.path(), "2026-10-21").unwrap(),
        "Goodnotes Import 2026-10-21 (2)"
    );
    assert_eq!(
        batch_name(&ONENOTE, home.path(), "2026-10-21").unwrap(),
        "OneNote Import 2026-10-21"
    );
    assert_eq!(today().len(), 10);
    library::validate_name("Goodnotes Import 2026-10-21 (2)").unwrap();
    library::validate_name("OneNote Import 2026-10-21").unwrap();
}

#[test]
fn only_one_update_or_import_can_claim_the_importer() {
    let importer = Arc::new(Importer::default());
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let importer = importer.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                importer.begin(Activity::Checking).is_ok()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert!(
        importer
            .begin(Activity::Importing {
                done: 0,
                total: 0,
                cancelling: false
            })
            .is_err()
    );
    importer.set(Activity::Idle);
    assert!(importer.begin(Activity::Installing).is_ok());
    // The other importer keeps its own job; only the fixed ids exist.
    let importers = Importers::default();
    let (goodnotes, tool) = importers.get("goodnotes").unwrap();
    assert_eq!(tool.id, "goodnotes");
    goodnotes.begin(Activity::Installing).unwrap();
    let (onenote, tool) = importers.get("onenote").unwrap();
    assert_eq!(tool.id, "onenote");
    assert!(onenote.begin(Activity::Installing).is_ok());
    assert!(importers.get("../goodnotes").is_err());
    assert!(importers.get("").is_err());
}

#[test]
fn catalog_exposes_only_frontend_metadata() {
    assert_eq!(
        serde_json::to_value(addon_catalog()).unwrap(),
        serde_json::json!([
            {
                "id": "goodnotes",
                "kind": "import",
                "i18nPrefix": "addons.goodnotes",
                "artwork": true,
            },
            {
                "id": "onenote",
                "kind": "import",
                "i18nPrefix": "addons.onenote",
                "artwork": false,
            },
            {
                "id": "math",
                "kind": "recognize",
                "i18nPrefix": "addons.math",
                "recognizes": "math",
            },
        ])
    );
}
