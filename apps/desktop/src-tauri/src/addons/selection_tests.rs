use super::*;

struct Book {
    _temp: tempfile::TempDir,
    root: PathBuf,
    history: storage::NotebookHistory,
}

fn book() -> Book {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/notebooks/minimal");
    let snapshot = storage::open_notebook(&fixture).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("book");
    fs::create_dir_all(root.join("references")).unwrap();
    fs::copy(
        fixture.join("references/lecture.pdf"),
        root.join("references/lecture.pdf"),
    )
    .unwrap();
    storage::create_notebook(&root, &snapshot).unwrap();
    let root = root.canonicalize().unwrap();
    let mut history = storage::NotebookHistory::default();
    storage::observe_notebook(&root, &mut history).unwrap();
    Book {
        _temp: temp,
        root,
        history,
    }
}

fn v(value: &impl Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

const PAGE: &str = "page-001";

fn captured(book: &Book) -> Capture {
    capture(&book.root, PAGE, vec!["stroke-001".into()]).unwrap()
}

fn batch(ops: Value) -> Value {
    json!({"batch":{"operations":ops}})
}

fn block(source: &str) -> Value {
    json!({"op":"addTypstBlock","source":source,"xPt":100,"yPt":220,"layoutWidthPt":230})
}

fn stage(
    proposals: &Proposals,
    capture: Capture,
    items: Vec<ProposalItem>,
    correction: Option<Operation>,
) -> String {
    let id = format!("p{}", proposals.pending.lock().unwrap().len());
    let review = Review {
        id: id.clone(),
        version: 1,
        root: capture.root.to_string_lossy().into_owned(),
        page_id: capture.base.page_id.clone(),
        status: "pending",
        addon: Name {
            en: "Math recognition".into(),
            de: "Mathematikerkennung".into(),
        },
        latex: correction.as_ref().map(|_| "\\foo".into()),
        items,
        anchor: capture.anchor.clone(),
        kind: "blocks",
    };
    proposals.pending.lock().unwrap().insert(
        id.clone(),
        Pending {
            review,
            capture,
            addon: &super::super::catalog::MATH,
            authorization: Authorization {
                version: "0.1.0".into(),
                path: PathBuf::from("math"),
                epoch: 0,
            },
            direct: false,
            correction,
        },
    );
    id
}

fn granted<'a>(_: &'a Pending) -> Result<Option<()>, StorageError> {
    Ok(Some(()))
}

#[test]
fn scope_carries_only_the_selected_page_points() {
    let book = book();
    let capture = captured(&book);
    let wire = serde_json::to_string(&capture.scope).unwrap();
    assert_eq!(capture.scope["kind"], "ink-selection");
    assert_eq!(
        capture.scope["page"],
        json!({"widthPt":capture.geometry.width_pt,"heightPt":capture.geometry.height_pt})
    );
    assert_eq!(capture.scope["strokes"][0]["id"], "s1");
    assert_eq!(capture.scope["strokes"][0]["widthPt"], 2.0);
    assert_eq!(
        capture.scope["strokes"][0]["points"][0],
        json!([80.0, 140.0])
    );
    assert_eq!(capture.scope.as_object().unwrap().len(), 3);
    for leaked in [PAGE, "stroke-001", "revision", "book", "pressure"] {
        assert!(!wire.contains(leaked), "{leaked} leaked into the scope");
    }
    // Forged, duplicate and empty selections are refused before dispatch.
    for ids in [
        vec!["forged".to_string()],
        vec!["stroke-001".into(), "stroke-001".into()],
        vec![],
        vec!["stroke-001".into(), "forged".into()],
    ] {
        assert!(capture_fails(&book, ids));
    }
}

fn capture_fails(book: &Book, ids: Vec<String>) -> bool {
    capture(&book.root, PAGE, ids).is_err()
}

#[test]
fn batch_validation_rejects_the_whole_batch() {
    let book = book();
    let capture = captured(&book);
    let geometry = &capture.geometry;
    assert_eq!(
        operations(batch(json!([block("$ x $")])), geometry)
            .unwrap()
            .len(),
        1
    );
    let mut wide = block("$ x $");
    wide["layoutWidthPt"] = json!(geometry.width_pt);
    let mut extra = block("$ x $");
    extra["id"] = json!("typst-001");
    let mut path = block("$ x $");
    path["sourcePath"] = json!("../escape.typ");
    let mut unknown = block("$ x $");
    unknown["op"] = json!("deleteObject");
    let mut negative = block("$ x $");
    negative["layoutWidthPt"] = json!(-1);
    let mut below = block("$ x $");
    below["yPt"] = json!(geometry.height_pt);
    for reply in [
        batch(json!([])),
        batch(json!((0..33).map(|_| block("$ x $")).collect::<Vec<_>>())),
        batch(json!([block("$ x $"), unknown])),
        batch(json!([block("$ x $"), extra])),
        batch(json!([path])),
        batch(json!([wide])),
        batch(json!([negative])),
        batch(json!([below])),
        batch(json!([block("   ")])),
        batch(json!([block(&"x".repeat(MAX_SOURCE + 1))])),
        batch(json!(["addTypstBlock"])),
        json!({"batch":{"operations":[block("$ x $")],"patch":[]}}),
        json!({"batch":{"operations":[block("$ x $")]},"extra":true}),
        json!({"operations":[block("$ x $")]}),
    ] {
        assert!(operations(reply.clone(), geometry).is_err(), "{reply}");
    }
    // Malformed Typst in one operation rejects the batch after compilation too.
    assert!(
        items(
            batch(json!([block("$ x^2 $"), block("#let")])),
            &capture,
            "a"
        )
        .is_err()
    );
    let compiled = items(batch(json!([block("$ x^2 $")])), &capture, "a").unwrap();
    assert_eq!(compiled.len(), 1);
    assert!(compiled[0].svg.contains("<svg"));
}

#[test]
fn reviewed_insert_is_one_undo_step_without_ai() {
    let mut book = book();
    let proposals = Proposals::default();
    let capture = captured(&book);
    let items = items(batch(json!([block("$ x^2 $")])), &capture, "a").unwrap();
    let before = storage::open_page(&book.root, PAGE).unwrap();
    let id = stage(&proposals, capture, items, None);

    // A declined direct apply writes nothing and keeps the proposal for review.
    let declined = proposals.commit(&book.root, PAGE, &mut book.history, &id, 1, |_| {
        Ok(None::<()>)
    });
    assert!(declined.unwrap().is_none());
    // A wrong version or destination is refused.
    assert!(
        proposals
            .commit(&book.root, PAGE, &mut book.history, &id, 2, granted)
            .is_err()
    );
    assert!(
        proposals
            .commit(&book.root, "page-002", &mut book.history, &id, 1, granted)
            .is_err()
    );

    let result = proposals
        .commit(&book.root, PAGE, &mut book.history, &id, 1, granted)
        .unwrap()
        .unwrap();
    assert_eq!(
        result.snapshot.page.objects.len(),
        before.page.objects.len() + 1
    );
    let added = result.snapshot.page.objects.last().unwrap();
    assert!(added.fields().id.starts_with("addon-a-"));
    assert_eq!(
        result.snapshot.page.reading_order.last(),
        Some(&added.fields().id)
    );
    assert_eq!(
        v(&storage::open_page(&book.root, PAGE).unwrap().ink_layers[0].strokes),
        v(&before.ink_layers[0].strokes)
    );
    let path = added.searchable_source().unwrap();
    assert_eq!(fs::read_to_string(book.root.join(path)).unwrap(), "$ x^2 $");
    // The proposal is consumed.
    assert!(
        proposals
            .commit(&book.root, PAGE, &mut book.history, &id, 1, granted)
            .is_err()
    );

    let undone = storage::undo_notebook(&book.root, &mut book.history).unwrap();
    assert_eq!(v(&undone.snapshot.page.objects), v(&before.page.objects));
    let redone = storage::redo_notebook(&book.root, &mut book.history).unwrap();
    assert_eq!(
        v(&redone.snapshot.page.objects),
        v(&result.snapshot.page.objects)
    );
    assert!(storage::redo_notebook(&book.root, &mut book.history).is_err());
}

#[test]
fn stale_revision_and_revoked_addon_are_refused() {
    let mut book = book();
    let proposals = Proposals::default();
    let capture = captured(&book);
    let items = items(batch(json!([block("$ x $")])), &capture, "a").unwrap();
    let id = stage(&proposals, capture, items, None);

    // Disabling, removing or updating the addon during the run revokes the proposal.
    let revoked = proposals.commit(&book.root, PAGE, &mut book.history, &id, 1, |_| {
        Err::<Option<()>, _>(invalid("addon-unavailable"))
    });
    assert!(revoked.is_err());
    assert!(proposals.pending.lock().unwrap().contains_key(&id));

    let mut changed = storage::open_page(&book.root, PAGE).unwrap();
    changed.ink_layers[0].strokes[0].points[0].x += 5.0;
    storage::commit_notebook(&book.root, &mut book.history, changed).unwrap();
    let before = storage::open_page(&book.root, PAGE).unwrap();
    let stale = proposals.commit(&book.root, PAGE, &mut book.history, &id, 1, granted);
    assert!(matches!(stale, Err(StorageError::ProposalStale(_))));
    assert!(!proposals.pending.lock().unwrap().contains_key(&id));
    assert_eq!(
        v(&storage::open_page(&book.root, PAGE).unwrap().page),
        v(&before.page)
    );
}

#[test]
fn unsupported_math_is_corrected_then_revalidated() {
    let mut book = book();
    let proposals = Proposals::default();
    let capture = captured(&book);
    let correction = Operation::AddTypstBlock {
        source: String::new(),
        x_pt: 80.0,
        y_pt: 150.0,
        layout_width_pt: 120.0,
    };
    let id = stage(&proposals, capture, vec![], Some(correction));
    // Nothing to insert until the writer supplies Typst.
    assert!(
        proposals
            .commit(&book.root, PAGE, &mut book.history, &id, 1, granted)
            .is_err()
    );
    assert!(proposals.edit(&id, 1, vec!["#let".into()]).is_err());
    assert!(proposals.edit(&id, 1, vec![]).is_err());
    assert!(
        proposals
            .edit(&id, 1, vec!["$ a $".into(), "$ b $".into()])
            .is_err()
    );
    let review = proposals.edit(&id, 1, vec!["$ alpha $".into()]).unwrap();
    assert_eq!(review.version, 2);
    assert!(review.latex.is_none());
    assert_eq!(review.items[0].source, "$ alpha $");
    // The old version is gone; a further edit bumps it again.
    assert!(proposals.edit(&id, 1, vec!["$ beta $".into()]).is_err());
    assert_eq!(
        proposals
            .edit(&id, 2, vec!["$ beta $".into()])
            .unwrap()
            .version,
        3
    );
    assert!(
        proposals
            .commit(&book.root, PAGE, &mut book.history, &id, 2, granted)
            .is_err()
    );
    let result = proposals
        .commit(&book.root, PAGE, &mut book.history, &id, 3, granted)
        .unwrap()
        .unwrap();
    let added = result.snapshot.page.objects.last().unwrap();
    assert_eq!(
        fs::read_to_string(book.root.join(added.searchable_source().unwrap())).unwrap(),
        "$ beta $"
    );
}

#[test]
fn unsupported_reply_needs_bounded_latex_only() {
    let error = |data: Value| session::CommandError {
        code: -32003,
        message: "Unsupported".into(),
        data: Some(data),
    };
    assert_eq!(
        unsupported_latex(error(json!({"latex":"\\x"}))).unwrap(),
        "\\x"
    );
    for data in [
        json!({"latex":"\\x","typst":"#exec"}),
        json!({"latex":1}),
        json!("\\x"),
        json!({"latex":"x".repeat(MAX_SOURCE + 1)}),
    ] {
        assert!(unsupported_latex(error(data)).is_err());
    }
    assert_eq!(command_problem(-32003), "addon-unsupported");
    assert_eq!(command_problem(-32002), "addon-cancelled");
    assert_eq!(command_problem(-32001), "addon-command-failed");
    assert_eq!(command_problem(-32603), "addon-command-failed");
}
