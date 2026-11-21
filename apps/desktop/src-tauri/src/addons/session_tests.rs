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
            json!({"apiVersion":API,"capabilities":capabilities,"addon":{"id":"math","version":"0.1.0"}}),
        );
    }
}

fn spawn(script: impl FnOnce(Fake) + Send + 'static) -> Result<Session, Failure> {
    spawn_with(script, || false)
}

fn spawn_with(
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
    Session::connect(
        host_in, host_out, host_err, None, "math", "0.1.0", cancelled,
    )
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
    let frames: Vec<Box<dyn Fn(&Value) -> String + Send>> = vec![
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
