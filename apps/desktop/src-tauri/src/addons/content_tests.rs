use super::*;
use serde_json::{Value, json};

const VERSION: &str = "1.2.3";
const SHAPES: &str = "content/shapes/circuits.json";
const TEMPLATE: &str = "content/templates/paper.json";
const PALETTE: &str = "content/palettes/pen.json";
const THEME: &str = "content/themes/desk.json";
const PACKAGE: &str = "content/typst/preview/zap/0.4.0";

fn declaration(contents: Value) -> Value {
    let mut value = serde_json::to_value(catalog::ENGINEERING.declaration(VERSION)).unwrap();
    value["contents"] = contents;
    for language in ["en", "de"] {
        value["strings"][language]["label"] = json!("Circuits");
        value["strings"][language]["search"] = json!("electronics circuit");
    }
    value
}

fn parse(value: &Value) -> Result<manifest::Manifest, manifest::Error> {
    manifest::parse(
        &serde_json::to_vec(value).unwrap(),
        &catalog::ENGINEERING,
        VERSION,
    )
}

fn shape_file() -> Value {
    json!({"id":"circuits","label":"label","items":[
        {"id":"resistor","label":"label","search":"search","widthPt":48,"heightPt":16,"objects":[
            {"type":"shape","x":0,"y":8,"geometry":{"kind":"line","start":{"x":0,"y":0},"end":{"x":10,"y":0}},"style":{}},
            {"type":"shape","x":10,"y":2,"geometry":{"kind":"rectangle","widthPt":28,"heightPt":12,"cornerRadiusPt":0},"style":{"fillColor":null}},
            {"type":"typst","x":14,"y":-5,"source":"$R_1$","layoutWidthPt":24,"scale":1}
        ]}
    ]})
}

fn write(root: &Path, file: &str, bytes: &[u8]) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn read_pack(root: &Path, declaration: &manifest::Manifest) -> Result<Pack, String> {
    load(
        &catalog::ENGINEERING,
        root,
        &declaration.strings,
        declaration.contents.as_ref().unwrap(),
    )
}

#[test]
fn transport_paths_counts_and_structure_are_strict() {
    let valid = declaration(json!({"shapes":[SHAPES]}));
    assert!(parse(&valid).is_ok());
    for (key, replacement) in [
        ("contents", json!({})),
        ("contents", json!({"shapes":[SHAPES], "extra":[]})),
        (
            "contents",
            json!({"shapes":[SHAPES, "content/shapes/CIRCUITS.json"]}),
        ),
        (
            "contents",
            json!({"shapes":["content/shapes/../untrusted.json"]}),
        ),
        ("contents", json!({"shapes":["content/templates/no.json"]})),
        (
            "contents",
            json!({"shapes":(0..33).map(|n| format!("content/shapes/{n}.json")).collect::<Vec<_>>()}),
        ),
        (
            "contents",
            json!({"typstPackages":["content/typst/local/zap/0.4.0"]}),
        ),
        ("entrypoint", Value::Null),
        ("entrypoint", json!("program.exe")),
        ("permissions", json!(["selected-ink"])),
        (
            "commands",
            json!([{"id":"run","label":"label","surface":"ink-selection"}]),
        ),
        ("transport", json!("stdio-jsonrpc")),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = replacement;
        assert!(parse(&invalid).is_err(), "{key}: {invalid}");
    }
    for addon in [&catalog::MATH, &catalog::GOODNOTES, &catalog::ONENOTE] {
        let existing = serde_json::to_vec(&addon.declaration("0.1.0")).unwrap();
        assert!(manifest::parse(&existing, addon, "0.1.0").is_ok());
        let mut existing: Value = serde_json::from_slice(&existing).unwrap();
        existing["contents"] = Value::Null;
        assert!(manifest::parse(&serde_json::to_vec(&existing).unwrap(), addon, "0.1.0").is_err());
    }
}

#[test]
fn contribution_validation_checks_structure_labels_and_caps() {
    let root = tempfile::tempdir().unwrap();
    let decl = parse(&declaration(json!({"shapes":[SHAPES]}))).unwrap();
    let valid = shape_file();
    write(root.path(), SHAPES, &serde_json::to_vec(&valid).unwrap());
    assert!(read_pack(root.path(), &decl).is_ok());
    for change in [
        ("/label", json!("missing")),
        ("/items/0/search", json!("missing")),
        ("/items/0/widthPt", json!(2)),
        ("/items/0/objects/0/style", json!({"strokeColor":"red"})),
        ("/items/0/objects/0/style", json!({"opacity":1.5})),
        ("/items/0/objects/0/style", json!({"unknown":false})),
        ("/items/0/objects/0/geometry/end", json!({"x":0,"y":0})),
        (
            "/items/0/objects/0/geometry/end",
            json!({"x":10,"y":0,"unknown":false}),
        ),
        ("/items/0/objects/2/source", json!("a".repeat(2049))),
        ("/items/0/objects/2/y", json!(-9)),
    ] {
        let mut value = valid.clone();
        *value.pointer_mut(change.0).unwrap() = change.1;
        write(root.path(), SHAPES, &serde_json::to_vec(&value).unwrap());
        assert!(read_pack(root.path(), &decl).is_err(), "{value}");
    }
    for bytes in [
        br#"{"id":"first","id":"second","label":"label","items":[]}"#.to_vec(),
        vec![b' '; MAX_CONTENT_JSON + 1],
    ] {
        write(root.path(), SHAPES, &bytes);
        assert!(read_pack(root.path(), &decl).is_err());
    }
    let oversized = json!({"id":"circuits","label":"label","items":[
        {"id":"large","label":"label","search":"search","widthPt":48,"heightPt":16,
            "objects":vec![valid["items"][0]["objects"][0].clone();65]}
    ]});
    write(
        root.path(),
        SHAPES,
        &serde_json::to_vec(&oversized).unwrap(),
    );
    assert!(read_pack(root.path(), &decl).is_err());
}

#[test]
fn palettes_templates_and_themes_validate_as_values() {
    let root = tempfile::tempdir().unwrap();
    let decl = parse(&declaration(
        json!({"templates":[TEMPLATE],"palettes":[PALETTE],"themes":[THEME]}),
    ))
    .unwrap();
    let template = json!({"templates":[{"label":"label","template":{
        "id":"engineering-shapes.graph","name":"Graph","backgroundColor":"#ffffff","elements":[]
    }}]});
    let palette = json!({"palettes":[{"id":"pen","label":"label","colors":["#123abc","#abcdef"]}]});
    let theme = json!({"themes":[{"id":"desk","label":"label","tokens":{"--panel":"#20242c","--accent-tint":"rgb(76 141 240 / 10%)"}}]});
    let files = [(TEMPLATE, template), (PALETTE, palette), (THEME, theme)];
    for (file, value) in &files {
        write(root.path(), file, &serde_json::to_vec(value).unwrap());
    }
    let loaded = read_pack(root.path(), &decl).unwrap();
    let definition = loaded.templates[0].template.clone();
    let background = goodtype_core::PageBackground::Template {
        template: definition.clone(),
    };
    drop(loaded);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/notebooks/minimal");
    let notebook = root.path().join("notebook");
    let source = storage::open_notebook(&fixture).unwrap();
    let reference = "references/lecture.pdf";
    write(
        &notebook,
        reference,
        &fs::read(fixture.join(reference)).unwrap(),
    );
    storage::create_notebook(&notebook, &source).unwrap();
    let new_page = storage::create_page(
        &notebook,
        "2026-11-23T00:00:00Z",
        &storage::PagePosition::Last,
        Some(&background),
        None,
    )
    .unwrap();
    fs::remove_file(root.path().join(TEMPLATE)).unwrap();
    let reopened = storage::open_page(&notebook, &new_page.page.id).unwrap();
    assert_eq!(
        reopened.page.background,
        goodtype_core::PageBackground::Template {
            template: definition
        }
    );
    write(
        root.path(),
        TEMPLATE,
        &serde_json::to_vec(&files[0].1).unwrap(),
    );
    for (file, path, bad) in [
        (TEMPLATE, "/templates/0/template/id", json!("other.graph")),
        (PALETTE, "/palettes/0/colors", json!(["#123abc", "#123ABC"])),
        (PALETTE, "/palettes/0/colors", json!(["#123abc", "#abcd"])),
        (THEME, "/themes/0/tokens", json!({"--paper":"#ffffff"})),
        (THEME, "/themes/0/tokens", json!({"--muted":"#23272f"})),
        (
            THEME,
            "/themes/0/tokens",
            json!({"--panel":"rgb(0 0 0 / 0%)"}),
        ),
        (
            THEME,
            "/themes/0/tokens",
            json!({"--on-accent":"rgb(0 0 0 / 1%)"}),
        ),
    ] {
        let good = &files.iter().find(|entry| entry.0 == file).unwrap().1;
        let mut value = good.clone();
        *value.pointer_mut(path).unwrap() = bad;
        write(root.path(), file, &serde_json::to_vec(&value).unwrap());
        assert!(read_pack(root.path(), &decl).is_err(), "{value}");
        write(root.path(), file, &serde_json::to_vec(good).unwrap());
    }
}

#[test]
fn verifies_install_without_probe_and_insertion_survives_removing_pack() {
    let temp = tempfile::tempdir().unwrap();
    let addon = &catalog::ENGINEERING;
    let top = package_folder(addon, VERSION);
    let archive = temp.path().join("pack.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for (path, bytes) in [
        (
            manifest::FILE,
            serde_json::to_vec(&declaration(json!({"shapes":[SHAPES]}))).unwrap(),
        ),
        (SHAPES, serde_json::to_vec(&shape_file()).unwrap()),
    ] {
        zip.start_file(
            format!("{top}/{path}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let hash = sha256_file(&archive).unwrap();
    let release = PinnedRelease {
        version: VERSION,
        url: "",
        size: fs::metadata(&archive).unwrap().len(),
        sha256: &hash,
        decoder_commit: "",
    };
    let dir = InstallDir(temp.path().join("installed"), addon);
    let loaded_version = install_archive(&dir, &archive, &release, &|_| {
        panic!("content pack launched a probe")
    })
    .unwrap();
    let (version, path) = dir.active().unwrap();
    assert_eq!(loaded_version, version);
    let installed = manager::installed(addon, &path, &version, true).unwrap();
    let pack = read_pack(&path, &installed).unwrap();

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/notebooks/minimal");
    let source = storage::open_notebook(&fixture).unwrap();
    let root = temp.path().join("book");
    let reference = "references/lecture.pdf";
    write(
        &root,
        reference,
        &fs::read(fixture.join(reference)).unwrap(),
    );
    storage::create_notebook(&root, &source).unwrap();
    let mut history = storage::NotebookHistory::default();
    storage::observe_notebook(&root, &mut history).unwrap();
    let mut request = ShapeInsertion {
        addon_id: addon.id.into(),
        library_id: "circuits".into(),
        item_id: "resistor".into(),
        revision: source.page.revision,
        point: Some(goodtype_core::ShapePoint { x: 80.0, y: 100.0 }),
        scale: Some(2.0),
        pen_color: "#223344".into(),
        pen_width_pt: 2.5,
    };
    let item = &pack.shapes[0].items[0];
    let compile = |source: &str, width_pt| {
        let result = goodtype_typst::compile_block_granted(
            &root,
            &goodtype_typst::CompileRequest {
                source: source.into(),
                width_pt,
                generation: 0,
                allow_remote_packages: false,
            },
            &goodtype_typst::GrantedFiles::new(),
        )
        .unwrap();
        result.width_pt.zip(result.height_pt)
    };
    let inserted = commit_shape(
        &root,
        &source.page.id,
        &mut history,
        &request,
        item,
        compile,
    )
    .unwrap();
    assert!(inserted.result.can_undo);
    let old_ids = inserted.selected_ids;
    assert_eq!(old_ids.len(), 3);
    let undone = storage::undo_notebook(&root, &mut history).unwrap();
    assert_eq!(
        undone.snapshot.page.objects.len(),
        source.page.objects.len()
    );
    request.revision = undone.snapshot.page.revision;
    let inserted = commit_shape(
        &root,
        &source.page.id,
        &mut history,
        &request,
        item,
        compile,
    )
    .unwrap();
    assert!(inserted.selected_ids.iter().all(|id| !old_ids.contains(id)));
    fs::remove_dir_all(&dir.0).unwrap();

    let reopened = storage::open_page(&root, &source.page.id).unwrap();
    let added = &reopened.page.objects[source.page.objects.len()..];
    let [
        PageObject::Shape { fields, style, .. },
        PageObject::Shape { .. },
        PageObject::Typst { .. },
        PageObject::Group { child_ids, .. },
    ] = added
    else {
        panic!("expected ordinary shapes, label, and group");
    };
    assert_eq!(child_ids, &inserted.selected_ids);
    assert_eq!((fields.x, fields.y, fields.scale), (32.0, 100.0, 2.0));
    assert_eq!(
        (style.stroke_color.as_str(), style.stroke_width_pt),
        ("#223344", 2.5)
    );
    let export = crate::export::export_page_from_bundle(&reopened, false).unwrap();
    assert!(goodtype_typst::export::export_pages(&root, "pack-check", &[export], false).is_ok());
}

#[test]
fn typst_package_identity_and_copy_are_checked() {
    let source = tempfile::tempdir().unwrap();
    let packages = tempfile::tempdir().unwrap();
    let decl = parse(&declaration(json!({"typstPackages":[PACKAGE]}))).unwrap();
    let toml = format!("{PACKAGE}/typst.toml");
    let entry = format!("{PACKAGE}/lib.typ");
    let valid = b"[package]\nname = \"zap\"\nversion = \"0.4.0\"\nentrypoint = \"lib.typ\"\n";
    write(source.path(), &entry, b"#let hello = [offline]");
    write(
        source.path(),
        &toml,
        b"[package]\nname = \"other\"\nversion = \"0.4.0\"\nentrypoint = \"lib.typ\"\n",
    );
    assert!(read_pack(source.path(), &decl).is_err());
    write(source.path(), &toml, valid);
    assert_eq!(
        read_pack(source.path(), &decl).unwrap().typst_packages,
        ["zap 0.4.0"]
    );
    let contents = decl.contents.as_ref().unwrap();
    retain_package_files(packages.path(), source.path(), contents).unwrap();
    write(source.path(), &entry, b"Different release");
    retain_package_files(packages.path(), source.path(), contents).unwrap();
    assert_eq!(
        fs::read(packages.path().join("preview/zap/0.4.0/lib.typ")).unwrap(),
        b"#let hello = [offline]"
    );
}

#[test]
fn content_archive_limits_apply_during_expansion() {
    let scratch = tempfile::tempdir().unwrap();
    for (path, size) in [("typst.dat", MAX_PACK_BYTES + 1), ("shape.json", MIB + 1)] {
        let file = tempfile::tempfile().unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            format!("pack/{path}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        io::copy(&mut io::repeat(0).take(size), &mut zip).unwrap();
        let mut file = zip.finish().unwrap();
        file.rewind().unwrap();
        assert!(
            extract_limited(
                &mut file,
                "pack",
                &scratch.path().join(path),
                MAX_PACK_BYTES
            )
            .is_err()
        );
    }
}

#[test]
fn spline_bounds_measure_the_curve_instead_of_its_handles() {
    assert!(cubic_inside([0.0, 12.0, 12.0, 0.0], 10.0));
    assert!(!cubic_inside([0.0, 16.0, 16.0, 0.0], 10.0));
    assert!(!cubic_inside([1.0, -100.0, 12.0, 1.0], 10.0));
}
