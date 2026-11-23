//! In-process fake addons over pipes. They exercise the same framing, drains and deadlines as a
//! launched child; only process exit detection is absent.
use super::*;
use std::io::{PipeReader, PipeWriter, pipe};
use std::sync::atomic::{AtomicBool, Ordering};

struct Fake {
    input: BufReader<PipeReader>,
    output: PipeWriter,
}

impl Fake {
    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.input.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn write(&mut self, frame: &str) {
        let _ = self.output.write_all(frame.as_bytes());
        let _ = self.output.write_all(b"\n");
    }

    fn reply(&mut self, id: &Value, result: Value) {
        let frame = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
        self.write(&frame);
    }

    fn initialize(&mut self, capabilities: &[&str]) {
        let request = self.read();
        assert_eq!(request["method"], "initialize");
        assert_eq!(request["params"]["apiVersion"], API);
        self.reply(
            &request["id"],
            json!({"apiVersion":API,"capabilities":capabilities,"addon":request["params"]["addon"]}),
        );
    }
}

type Frame = Box<dyn Fn(&Value) -> String + Send>;

fn spawn(script: impl FnOnce(Fake) + Send + 'static) -> Result<Session, Failure> {
    spawn_with(script, || false)
}

fn spawn_with(
    script: impl FnOnce(Fake) + Send + 'static,
    cancelled: impl Fn() -> bool,
) -> Result<Session, Failure> {
    spawn_for("math", script, cancelled)
}

fn spawn_for(
    id: &str,
    script: impl FnOnce(Fake) + Send + 'static,
    cancelled: impl Fn() -> bool,
) -> Result<Session, Failure> {
    let (child_in, host_in) = pipe().unwrap();
    let (host_out, child_out) = pipe().unwrap();
    let (host_err, mut child_err) = pipe().unwrap();
    std::thread::spawn(move || {
        let _ = child_err.write_all(&vec![b'e'; 40 * 1024]);
        script(Fake {
            input: BufReader::new(child_in),
            output: child_out,
        });
    });
    Session::connect(host_in, host_out, host_err, None, id, "0.1.0", cancelled)
}

fn scope() -> Value {
    json!({"command":"recognize-math","scope":{"kind":"ink-selection",
        "page":{"widthPt":595.28,"heightPt":841.89},
        "strokes":[{"id":"s1","widthPt":2.0,"points":[[100.0,200.0],[110.0,205.0]]}]},"settings":{}})
}

fn batch() -> Value {
    json!({"batch":{"operations":[{"op":"addTypstBlock","source":"$ x^2 $","xPt":100,"yPt":220,"layoutWidthPt":230}]}})
}

fn call(session: &mut Session, timeout: Duration) -> Result<Result<Value, CommandError>, Failure> {
    session.call("command/invoke", scope(), timeout, || false)
}

#[test]
fn handshake_and_invoke_match_the_contract_wire() {
    let mut session = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0", "cancel"]);
        let request = fake.read();
        assert_eq!(request["id"], "2");
        assert_eq!(request["params"], scope());
        fake.reply(&request["id"], batch());
        // Keep the pipes open so the session stays healthy.
        let _ = fake.read();
    })
    .unwrap();
    assert_eq!(
        call(&mut session, Duration::from_secs(5)).unwrap().unwrap(),
        batch()
    );
    std::thread::sleep(Duration::from_millis(50));
    assert!(session.healthy());
    // The stderr drain keeps at most 16 KiB and never blocks the child.
    assert_eq!(session.diagnostics().len(), STDERR_LIMIT);
}

#[test]
fn handshake_mismatch_needs_update() {
    for reply in [
        json!({"apiVersion":"goodtype-api 0.2","capabilities":["page-batch-v0"],"addon":{"id":"math","version":"0.1.0"}}),
        json!({"apiVersion":API,"capabilities":["page-batch-v0"],"addon":{"id":"math","version":"0.1.1"}}),
        json!({"apiVersion":API,"capabilities":["page-batch-v0"],"addon":{"id":"onenote","version":"0.1.0"}}),
        json!({"apiVersion":API,"capabilities":["cancel"],"addon":{"id":"math","version":"0.1.0"}}),
        json!({"apiVersion":API,"capabilities":["page-batch-v0","page-batch-v0"],"addon":{"id":"math","version":"0.1.0"}}),
        json!({"apiVersion":API,"capabilities":["page-batch-v0","future"],"addon":{"id":"math","version":"0.1.0"}}),
        json!({"apiVersion":API,"capabilities":["page-batch-v0"],"addon":{"id":"math","version":"0.1.0"},"extra":1}),
    ] {
        let result = spawn(move |mut fake| {
            let request = fake.read();
            fake.reply(&request["id"], reply);
        });
        assert_eq!(result.err(), Some(Failure::NeedsUpdate));
    }
    let refused = spawn(|mut fake| {
        let request = fake.read();
        let frame = json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32602,"message":"Incompatible"}});
        fake.write(&frame.to_string());
    });
    assert_eq!(refused.err(), Some(Failure::NeedsUpdate));
}

#[test]
fn invalid_frames_invalidate_the_session() {
    let frames: Vec<Frame> = vec![
        // Oversized.
        Box::new(|_| " ".repeat(MAX_FRAME + 1)),
        // Duplicate key.
        Box::new(|id| format!(r#"{{"jsonrpc":"2.0","id":{id},"id":{id},"result":{{}}}}"#)),
        // Wrong ID.
        Box::new(|_| r#"{"jsonrpc":"2.0","id":"9","result":{}}"#.into()),
        // Numeric ID.
        Box::new(|_| r#"{"jsonrpc":"2.0","id":2,"result":{}}"#.into()),
        // Both result and error.
        Box::new(|id| {
            format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{{}},"error":{{"code":1,"message":""}}}}"#
            )
        }),
        // Wrong version.
        Box::new(|id| format!(r#"{{"jsonrpc":"1.0","id":{id},"result":{{}}}}"#)),
        // Child-initiated request.
        Box::new(|_| r#"{"jsonrpc":"2.0","id":"7","method":"fs/read","params":{}}"#.into()),
        // Unexpected notification and progress without negotiation.
        Box::new(|_| r#"{"jsonrpc":"2.0","method":"log","params":{}}"#.into()),
        Box::new(|id| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"command/progress","params":{{"id":{id},"phase":"recognizing","completed":1,"total":2}}}}"#
            )
        }),
        // Batch.
        Box::new(|_| "[]".into()),
        // Malformed UTF-8 is rejected by the decoder; a lone surrogate escape too.
        Box::new(|id| format!(r#"{{"jsonrpc":"2.0","id":{id},"result":"\ud800"}}"#)),
        // Overlong diagnostic.
        Box::new(|id| {
            format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32001,"message":"{}"}}}}"#,
                "m".repeat(513)
            )
        }),
    ];
    for frame in frames {
        let mut session = spawn(move |mut fake| {
            fake.initialize(&["page-batch-v0"]);
            let request = fake.read();
            fake.write(&frame(&request["id"]));
            let _ = fake.read();
        })
        .unwrap();
        assert_eq!(
            call(&mut session, Duration::from_secs(5)).err(),
            Some(Failure::Protocol)
        );
    }
    let mut invalid_utf8 = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let _ = fake.read();
        let _ = fake
            .output
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"2\",\"result\":\"\xff\"}\n");
        let _ = fake.read();
    })
    .unwrap();
    assert_eq!(
        call(&mut invalid_utf8, Duration::from_secs(5)).err(),
        Some(Failure::Protocol)
    );
}

#[test]
fn unterminated_frame_and_crash_invalidate_the_session() {
    let mut unterminated = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let _ = fake.read();
        let _ = fake
            .output
            .write_all(br#"{"jsonrpc":"2.0","id":"2","result":{}}"#);
    })
    .unwrap();
    assert_eq!(
        call(&mut unterminated, Duration::from_secs(5)).err(),
        Some(Failure::Protocol)
    );
    let mut crashed = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let _ = fake.read();
    })
    .unwrap();
    assert_eq!(
        call(&mut crashed, Duration::from_secs(5)).err(),
        Some(Failure::Protocol)
    );
    assert!(!crashed.healthy());
    // A restart is a fresh session with a fresh handshake.
    let mut restarted = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let request = fake.read();
        assert_eq!(request["id"], "2");
        fake.reply(&request["id"], batch());
        let _ = fake.read();
    })
    .unwrap();
    assert!(call(&mut restarted, Duration::from_secs(5)).is_ok());
}

#[test]
fn command_error_keeps_the_session_usable() {
    let mut session = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let request = fake.read();
        let frame = json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32003,"message":"Unsupported","data":{"latex":"\\foo"}}});
        fake.write(&frame.to_string());
        let request = fake.read();
        assert_eq!(request["id"], "3");
        fake.reply(&request["id"], batch());
        let _ = fake.read();
    })
    .unwrap();
    let error = call(&mut session, Duration::from_secs(5))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, -32003);
    assert_eq!(
        super::super::selection::unsupported_latex(error).unwrap(),
        "\\foo"
    );
    assert!(session.healthy());
    assert!(call(&mut session, Duration::from_secs(5)).unwrap().is_ok());
}

#[test]
fn progress_is_bounded_and_does_not_extend_the_deadline() {
    let mut session = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0", "progress"]);
        let request = fake.read();
        let id = request["id"].clone();
        for completed in 0..2_000 {
            let frame = json!({"jsonrpc":"2.0","method":"command/progress","params":{"id":id,"phase":"recognizing","completed":completed % 3,"total":2}});
            fake.write(&frame.to_string());
        }
        fake.reply(&id, batch());
        let request = fake.read();
        let frame = json!({"jsonrpc":"2.0","method":"command/progress","params":{"id":request["id"],"phase":"recognizing","completed":3,"total":2}});
        fake.write(&frame.to_string());
        let _ = fake.read();
    })
    .unwrap();
    assert!(call(&mut session, Duration::from_secs(5)).unwrap().is_ok());
    // `completed > total` is a protocol violation.
    assert_eq!(
        call(&mut session, Duration::from_secs(5)).err(),
        Some(Failure::Protocol)
    );

    let flood = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0", "progress"]);
        let request = fake.read();
        loop {
            let frame = json!({"jsonrpc":"2.0","method":"command/progress","params":{"id":request["id"],"phase":"recognizing","completed":1,"total":2}});
            if fake
                .output
                .write_all(format!("{frame}\n").as_bytes())
                .is_err()
            {
                break;
            }
        }
    });
    let mut flood = flood.unwrap();
    let started = Instant::now();
    assert_eq!(
        call(&mut flood, Duration::from_millis(300)).err(),
        Some(Failure::Timeout)
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn silent_child_times_out() {
    let mut session = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let _ = fake.read();
        let _ = fake.read();
    })
    .unwrap();
    assert_eq!(
        call(&mut session, Duration::from_millis(200)).err(),
        Some(Failure::Timeout)
    );
}

#[test]
fn cancel_notifies_then_discards_a_late_completion() {
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&saw_cancel);
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&flag);
    let mut session = spawn(move |mut fake| {
        fake.initialize(&["page-batch-v0", "cancel"]);
        let request = fake.read();
        trigger.store(true, Ordering::SeqCst);
        let cancel = fake.read();
        assert_eq!(
            cancel,
            json!({"jsonrpc":"2.0","method":"command/cancel","params":{"id":request["id"]}})
        );
        observed.store(true, Ordering::SeqCst);
        // The result races the cancellation and must not be published.
        fake.reply(&request["id"], batch());
        let _ = fake.read();
    })
    .unwrap();
    let started = Instant::now();
    let result = session.call("command/invoke", scope(), Duration::from_secs(5), || {
        flag.load(Ordering::SeqCst)
    });
    assert_eq!(result.err(), Some(Failure::Cancelled));
    assert!(saw_cancel.load(Ordering::SeqCst));
    assert!(started.elapsed() < Duration::from_secs(3));

    // Without the negotiated capability no notification is sent before the session ends.
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&flag);
    let (report, rest) = mpsc::channel();
    let mut quiet = spawn(move |mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let _ = fake.read();
        trigger.store(true, Ordering::SeqCst);
        let mut line = String::new();
        let _ = fake.input.read_line(&mut line);
        let _ = report.send(line);
    })
    .unwrap();
    assert_eq!(
        quiet
            .call("command/invoke", scope(), Duration::from_secs(5), || {
                flag.load(Ordering::SeqCst)
            })
            .err(),
        Some(Failure::Cancelled)
    );
    drop(quiet);
    assert_eq!(rest.recv_timeout(Duration::from_secs(5)).unwrap(), "");
    // Cancellation before the handshake completes never starts a command.
    assert_eq!(
        spawn_with(
            |mut fake| {
                let _ = fake.read();
            },
            || true
        )
        .err(),
        Some(Failure::Cancelled)
    );
}

#[test]
fn unsolicited_output_and_the_request_cap_retire_a_warm_worker() {
    let mut session = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        fake.write(r#"{"jsonrpc":"2.0","id":"1","result":{}}"#);
        let _ = fake.read();
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!session.healthy());

    let mut capped = spawn(|mut fake| {
        fake.initialize(&["page-batch-v0"]);
        let _ = fake.read();
    })
    .unwrap();
    assert!(capped.healthy());
    capped.next_id = MAX_REQUESTS - 1;
    assert!(capped.healthy());
    capped.next_id = MAX_REQUESTS;
    assert!(!capped.healthy());
    capped.next_id = 1;
    capped.used = Instant::now() - Duration::from_secs(601);
    assert!(!capped.healthy());
}

/// A launcher refusal (here a junction inside the install tree) must surface as the sandbox
/// failure itself. A fallback launch would instead fail later at the handshake.
#[cfg(windows)]
#[test]
fn sandbox_launch_failure_makes_the_addon_unavailable_without_fallback() {
    let home = tempfile::tempdir().unwrap();
    let install = home.path().join("install");
    let scratch = home.path().join("sessions");
    for dir in [&install, &scratch] {
        std::fs::create_dir(dir).unwrap();
    }
    let executable = install.join("goodtype-math-recognize.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let linked = install.join("linked");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&linked)
        .arg(home.path())
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    // The install tree is inspected once, when its stable profile is granted.
    let result = ensure_grant(&scratch, "math", "0.1.0", &install);
    assert_eq!(result.err(), Some(Failure::Sandbox));
    assert_eq!(Failure::Sandbox.code(), "addon-sandbox-failed");
    // No session scratch was created, and the marker stays "pending" for the startup sweep.
    let entries: Vec<_> = std::fs::read_dir(&scratch)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["grants"]);
    let marker = std::fs::read_to_string(scratch.join("grants/Goodtype.Addon.math.0.1.0")).unwrap();
    assert!(
        marker.starts_with(
            "pending
"
        ),
        "{marker}"
    );
    // A launch that names the profile never inspects or grants the install again, but the
    // launcher still refuses an executable inside scratch.
    let inside = scratch.join("session-x");
    std::fs::create_dir(&inside).unwrap();
    std::fs::write(inside.join("a.exe"), b"MZ").unwrap();
    assert!(
        crate::sandbox::launch(crate::sandbox::SandboxSpec {
            executable: &inside.join("a.exe"),
            args: &[],
            cwd: &inside,
            env: crate::sandbox::minimal_environment().unwrap(),
            read_only: &[],
            scratch: Some(&inside),
            capabilities: &[],
            profile: None,
            memory_limit: 64 << 20,
            active_process_limit: 1,
            stdio: false,
        })
        .is_err()
    );
    std::fs::remove_dir(&linked).unwrap();
}

/// Runs the real local math package: `GOODTYPE_MATH_ZIP=<zip> cargo test -- --ignored
/// native_math_session`. Only release-signature verification is bypassed; the inventory,
/// manifest and handshake are the production checks.
#[test]
#[ignore = "needs a local Goodtype math addon package"]
fn native_math_session_recognizes_a_selection() {
    let zip = std::env::var("GOODTYPE_MATH_ZIP").unwrap_or_else(|_| {
        r"C:\Users\jugri\Documents\Programieren\Goodtype\Goodtype-Math-Recognizer\target\package\goodtype-math-recognizer-0.1.0-local.20261121121521-windows-x64.zip".into()
    });
    let top = Path::new(&zip)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("math");
    super::super::extract_package(&mut std::fs::File::open(&zip).unwrap(), &top, &dir).unwrap();
    let math = &super::super::catalog::MATH;
    let inventory = super::super::verify_package(math, &dir).unwrap();
    // A local build's inventory carries a prerelease version ("0.1.0-local.<stamp>"), which a
    // release install refuses; the manifest itself declares the base version.
    let version = inventory.version.split('-').next().unwrap().to_owned();
    let bytes = std::fs::read(dir.join(super::super::manifest::FILE)).unwrap();
    let manifest = super::super::manifest::parse(&bytes, math, &version).unwrap();
    assert!(manifest.supports_selection("recognize-math"));

    let scratch = home.path().join("sessions");
    std::fs::create_dir(&scratch).unwrap();
    let granting = Instant::now();
    let profile = ensure_grant(&scratch, math.id, &version, &dir).unwrap();
    let grant = granting.elapsed();
    let start = || {
        let started = Instant::now();
        let session = Session::start(
            &dir.join(math.executable()),
            &scratch,
            &profile,
            math.id,
            &version,
            || false,
        )
        .unwrap();
        (session, started.elapsed())
    };
    let (first, first_handshake) = start();
    drop(first);
    // A later session reuses the grant: only its fresh scratch gets an ACE.
    assert_eq!(
        ensure_grant(&scratch, math.id, &version, &dir).unwrap(),
        profile
    );
    let (mut session, handshake) = start();
    // A handwritten "x": two crossing diagonals.
    let scope = json!({"command":"recognize-math","scope":{"kind":"ink-selection",
        "page":{"widthPt":595.28,"heightPt":841.89},
        "strokes":[
            {"id":"s1","widthPt":2.0,"points":(0..20).map(|i| [100.0 + i as f64, 200.0 + i as f64]).collect::<Vec<_>>()},
            {"id":"s2","widthPt":2.0,"points":(0..20).map(|i| [119.0 - i as f64, 200.0 + i as f64]).collect::<Vec<_>>()}
        ]},"settings":{}});
    let cold = Instant::now();
    let reply = session.invoke(scope.clone(), || false).unwrap();
    let cold = cold.elapsed();
    let warm = Instant::now();
    let again = session.invoke(scope, || false).unwrap();
    let warm = warm.elapsed();
    let geometry = goodtype_core::PageGeometry {
        width_pt: 595.28,
        height_pt: 841.89,
    };
    for reply in [reply, again] {
        match reply {
            Ok(batch) => {
                let ops = super::super::selection::operations(batch.clone(), &geometry).unwrap();
                eprintln!("batch {batch}");
                assert_eq!(ops.len(), 1);
            }
            Err(error) => {
                assert_eq!(error.code, -32003);
                eprintln!(
                    "unsupported {:?}",
                    super::super::selection::unsupported_latex(error).unwrap()
                );
            }
        }
    }
    assert!(session.healthy());
    drop(session);
    eprintln!(
        "grant once {grant:?}, first handshake {first_handshake:?}, next handshake {handshake:?}, cold invoke {cold:?}, warm invoke {warm:?}"
    );
    revoke_where(&scratch, |_, _| false);
    assert!(!scratch.join("grants").join(&profile).exists());
}

#[test]
fn fixture_contributions_use_the_session_protocol() {
    use crate::addons::{
        batch as page_batch,
        contributions::{Invocation, defaults, fixture},
        manifest::Surface,
    };
    use goodtype_core::{PageGeometry, storage};
    use std::path::Path;
    let fixture = fixture();
    let insert = &fixture.commands[0];
    let generator = &fixture.commands[1];
    let export = &fixture.commands[2];
    let mut settings = defaults(&fixture.settings);
    settings.insert("outline".into(), true.into());
    let geometry = PageGeometry {
        width_pt: 595.0,
        height_pt: 842.0,
    };
    let form = |surface, values| Invocation::Form {
        surface,
        values,
        geometry: geometry.clone(),
        anchor: [10.0, 10.0],
    };
    let values = defaults(insert.form.as_ref().unwrap());
    for (field, bad) in [
        ("title", json!("a".repeat(41))),
        ("count", json!(20)),
        ("color", json!("missing")),
        ("count", json!(2)),
    ] {
        let mut invalid = values.clone();
        invalid.insert(field.into(), bad);
        assert_eq!(
            form(Surface::InsertForm, invalid).params(insert, settings.clone()),
            Err(Failure::InvalidForm)
        );
    }
    let shape = json!({"op":"addShape","x":30,"y":80,"geometry":{"kind":"rectangle","widthPt":140,"heightPt":50,"cornerRadiusPt":4},"style":{"strokeColor":"#182c39","strokeWidthPt":2,"fillColor":"#e9e6dd","opacity":1}});
    let insert_reply = json!({"batch":{"operations":[{"op":"addGroup","children":[shape]}]}});
    let generator_reply = json!({"batch":{"operations":[{"op":"addPage","template":null,"ops":[shape]},{"op":"addPage","template":null,"ops":[]}]}});
    let mut worker = spawn_for("fixture", {
        let insert_reply = insert_reply.clone();
        let generator_reply = generator_reply.clone();
        move |mut fake| {
            fake.initialize(&["page-batch-v0"]);
            let input = fake.read();
            assert_eq!(input["params"], json!({"command":"insert-diagram","settings":{"outline":true},"form":{"title":"Parabola","count":1.0,"color":"blue"},"page":{"widthPt":595.0,"heightPt":842.0},"anchor":{"x":10.0,"y":10.0}}));
            fake.reply(&input["id"], insert_reply);
            let input = fake.read();
            assert_eq!(input["params"]["command"], "worksheet");
            fake.reply(&input["id"], generator_reply);
            let input = fake.read();
            assert_eq!(input["params"]["command"], "export-diagrams");
            assert!(input["params"]["pages"].is_array());
            fake.reply(&input["id"], json!({"bytes":"b2s="}));
        }
    }, || false).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fixture.goodtype");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/notebooks/minimal");
    let mut snapshot = storage::open_notebook(&fixture).unwrap();
    let background = goodtype_core::PageBackground::Plain {
        color: "#ffffff".into(),
    };
    snapshot.page.background = background.clone();
    snapshot.manifest.default_page.background = background;
    snapshot.manifest.shared_style_path = None;
    snapshot.page.objects.clear();
    snapshot.page.reading_order.clear();
    snapshot.blocks.clear();
    snapshot.assets.clear();
    for layer in &mut snapshot.ink_layers {
        layer.strokes.clear();
    }
    storage::create_notebook(&root, &snapshot).unwrap();
    let mut history = storage::NotebookHistory::default();
    let before = storage::observe_notebook(&root, &mut history).unwrap();
    let params = form(Surface::InsertForm, values)
        .params(insert, settings.clone())
        .unwrap();
    let reply = worker.invoke(params, || false).unwrap().unwrap();
    let mut prepared = page_batch::prepare(&root, &geometry, reply, false, || false).unwrap();
    use crate::addons::contributions::commit_allowed;
    assert!(!commit_allowed(true, false, || true));
    assert!(!commit_allowed(true, true, || false));
    assert!(commit_allowed(true, true, || true));
    assert!(commit_allowed(false, false, || false));
    assert!(
        storage::open_notebook(&root)
            .unwrap()
            .page
            .objects
            .is_empty()
    );
    let mut candidate = before.clone();
    prepared.remove(0).insert(&mut candidate).unwrap();
    let inserted = storage::commit_notebook(&root, &mut history, candidate).unwrap();
    assert_eq!(inserted.snapshot.page.objects.len(), 2);
    assert!(
        storage::undo_notebook(&root, &mut history)
            .unwrap()
            .snapshot
            .page
            .objects
            .is_empty()
    );

    let params = form(Surface::PageGenerator, defaults(&[]))
        .params(generator, settings.clone())
        .unwrap();
    let reply = worker.invoke(params, || false).unwrap().unwrap();
    let pages = page_batch::prepare(&root, &geometry, reply, true, || false).unwrap();
    assert_eq!(
        storage::open_notebook(&root).unwrap().manifest.pages.len(),
        1
    );
    let mut history = storage::NotebookStructureHistory::default();
    storage::observe_structure(&root, &before.page.id, &mut history).unwrap();
    let pages = pages
        .into_iter()
        .map(page_batch::PreviewPage::generated)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let added = storage::add_generated_pages(
        &root,
        "2026-11-23T12:01:00Z",
        &storage::PagePosition::Last,
        &geometry,
        pages,
    )
    .unwrap();
    assert_eq!(added.manifest.pages.len(), 3);
    assert_eq!(added.page.objects.len(), 1);
    storage::advance_structure(&root, &added, &mut history).unwrap();
    assert_eq!(
        storage::undo_structure(&root, "2026-11-23T12:02:00Z", &mut history)
            .unwrap()
            .snapshot
            .manifest
            .pages
            .len(),
        1
    );

    assert!(
        !crate::addons::export::after_destination(None, |_| panic!(
            "cancelled export invoked the addon"
        ))
        .unwrap()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    let pages = crate::addons::export::export_pages(&root, None, false).unwrap();
    let params = Invocation::Export {
        assets: false,
        pages,
    }
    .params(export, settings)
    .unwrap();
    let reply = worker.invoke(params, || false).unwrap().unwrap();
    let chosen = dir.path().join("chosen.json");
    assert!(
        crate::addons::export::after_destination(Some(chosen.clone()), |path| {
            crate::addons::export::write_reply(&path, reply)
        })
        .unwrap()
    );
    assert_eq!(std::fs::read(&chosen).unwrap(), b"ok");
    assert!(crate::addons::export::write_reply(&chosen, json!({"bytes":"invalid!"})).is_err());
    assert_eq!(std::fs::read(&chosen).unwrap(), b"ok");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
