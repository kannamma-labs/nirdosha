#![cfg(feature = "native")]
use nirdosha_rt::native::{Authority, AuthorizedSaga, DurableSaga, Outcome, Saga};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const JWKS: &str = r#"{"keys":[{"kid":"key1","kty":"oct","k":"bXktc2VjcmV0LWtleQ"}]}"#;
fn authority() -> Authority {
    Authority::new("issuer".into(), "audience".into(), JWKS.into()).unwrap()
}
fn token(roles: Value, expiry: i64, secret: &[u8]) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    header.kid = Some("key1".into());
    jsonwebtoken::encode(&header, &json!({"iss":"issuer","aud":"audience","sub":"alice","iat":100,"exp":expiry,"roles":roles}), &jsonwebtoken::EncodingKey::from_secret(secret)).unwrap()
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nir-native-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn saga(&self) -> DurableSaga<Ledger> {
        DurableSaga::open(
            &self.0.join("log.sqlite"),
            Ledger(self.0.join("effects.sqlite")),
        )
        .unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// External provider simulator with a REAL durable idempotency key. Inserting
/// the receipt and applying the business effect happen in one SQLite commit.
struct Ledger(PathBuf);
impl Ledger {
    fn effect(&self, id: &str, step: &str) -> Result<(), String> {
        let mut conn = rusqlite::Connection::open(&self.0).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS receipts (id TEXT, step TEXT, PRIMARY KEY(id,step)); CREATE TABLE IF NOT EXISTS balances (step TEXT PRIMARY KEY, value INTEGER NOT NULL);").map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let added = tx
            .execute("INSERT OR IGNORE INTO receipts VALUES (?, ?)", [id, step])
            .map_err(|e| e.to_string())?;
        if added == 1 {
            tx.execute(
                "INSERT INTO balances VALUES (?, 1) ON CONFLICT(step) DO UPDATE SET value=value+1",
                [step],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
}
impl Saga for Ledger {
    fn protocol(&self) -> &'static str {
        "ledger-transfer/v1"
    }
    fn network(&mut self, id: &str, _input: &Value) -> Result<Value, String> {
        self.effect(id, "network")?;
        Ok(json!({"receipt": id}))
    }
    fn verify(&self, input: &Value, _result: &Value) -> Result<bool, String> {
        Ok(input["accept"] == true)
    }
    fn commit(&mut self, id: &str, _input: &Value, _result: &Value) -> Result<(), String> {
        self.effect(id, "commit")
    }
    fn compensate(&mut self, id: &str, _input: &Value, _result: &Value) -> Result<(), String> {
        self.effect(id, "compensate")
    }
}

#[test]
fn authentication_checks_signature_authority_roles_and_expiry() {
    let authority = authority();
    let good = token(json!(["finance"]), 200, b"my-secret-key");
    let identity = authority.verify(&good, 150).unwrap();
    assert_eq!(
        authority
            .authorize(&identity, "finance", 150)
            .unwrap()
            .subject(),
        "alice"
    );
    assert!(authority.authorize(&identity, "admin", 150).is_err());
    assert!(authority.authorize(&identity, "finance", 200).is_err());
    let other_authority = Authority::new("issuer".into(), "audience".into(), JWKS.into()).unwrap();
    assert!(
        other_authority
            .authorize(&identity, "finance", 150)
            .is_err()
    );
    assert!(authority.verify(&good, 99).is_err());
    assert!(
        authority
            .verify(&token(json!(["finance"]), 200, b"wrong-key"), 150)
            .is_err()
    );
    assert!(
        authority
            .verify(&token(json!("finance"), 200, b"my-secret-key"), 150)
            .is_err()
    );
    assert!(
        Authority::new("wrong-issuer".into(), "audience".into(), JWKS.into())
            .unwrap()
            .verify(&good, 150)
            .is_err()
    );
}

#[test]
fn denied_requests_create_no_transaction_or_business_effect() {
    let directory = Directory::new();
    let mut service = AuthorizedSaga::new(authority(), "finance".into(), directory.saga()).unwrap();
    for bad in [
        token(json!(["janitor"]), 200, b"my-secret-key"),
        token(json!(["finance"]), 149, b"my-secret-key"),
        token(json!(["finance"]), 200, b"wrong-key"),
    ] {
        assert!(
            service
                .execute(&bad, 150, "transfer", &json!({"accept":true}))
                .is_err()
        );
    }
    let conn = rusqlite::Connection::open(directory.0.join("log.sqlite")).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM nirdosha_transact_log", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert!(!directory.0.join("effects.sqlite").exists());
    let good = token(json!(["finance"]), 200, b"my-secret-key");
    assert_eq!(
        service
            .execute(&good, 150, "transfer", &json!({"accept":true}))
            .unwrap(),
        Outcome::Committed
    );
    assert_eq!(
        service
            .execute(&good, 150, "transfer", &json!({"accept":true}))
            .unwrap(),
        Outcome::Committed
    );
    assert!(
        service
            .execute(&good, 150, "transfer", &json!({"accept":false}))
            .is_err()
    );
    assert_balances(&directory, "commit");
}

fn assert_balances(directory: &Directory, terminal: &str) {
    let conn = rusqlite::Connection::open(directory.0.join("effects.sqlite")).unwrap();
    let mut query = conn
        .prepare("SELECT step,value FROM balances ORDER BY step")
        .unwrap();
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut expected = vec![("network".to_owned(), 1), (terminal.to_owned(), 1)];
    expected.sort();
    assert_eq!(rows, expected);
}

#[test]
fn crash_worker() {
    let Ok(path) = std::env::var("NIR_TEST_CRASH_DIRECTORY") else {
        return;
    };
    let path = PathBuf::from(path);
    let stage = std::env::var("NIR_TEST_CRASH_STAGE").unwrap();
    let accept = std::env::var("NIR_TEST_ACCEPT").unwrap() == "true";
    let mut saga = DurableSaga::open(
        &path.join("log.sqlite"),
        Ledger(path.join("effects.sqlite")),
    )
    .unwrap();
    saga.execute_observed("transfer", &json!({"accept":accept}), |at| {
        if at == stage {
            println!("CRASH_BOUNDARY_READY");
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
    })
    .unwrap();
    panic!("requested crash boundary was never reached");
}

#[test]
fn kill_and_restart_at_every_commit_and_compensation_boundary() {
    for (accept, stages, terminal) in [
        (
            true,
            vec![
                "pending",
                "network_effect",
                "network_done",
                "commit_pending",
                "commit_effect",
                "committed",
            ],
            "commit",
        ),
        (
            false,
            vec![
                "pending",
                "network_effect",
                "network_done",
                "compensate_pending",
                "compensate_effect",
                "compensated",
            ],
            "compensate",
        ),
    ] {
        for stage in stages {
            let directory = Directory::new();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "crash_worker", "--nocapture"])
                .env("NIR_TEST_CRASH_DIRECTORY", &directory.0)
                .env("NIR_TEST_CRASH_STAGE", stage)
                .env("NIR_TEST_ACCEPT", accept.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let (sender, receiver) = mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if line.unwrap_or_default().contains("CRASH_BOUNDARY_READY") {
                        let _ = sender.send(());
                        break;
                    }
                }
            });
            let ready = receiver.recv_timeout(Duration::from_secs(20));
            let _ = child.kill();
            child.wait().unwrap();
            reader.join().unwrap();
            assert!(ready.is_ok(), "worker never reached {stage}");
            let mut recovered = directory.saga();
            recovered.recover().unwrap();
            let outcome = recovered
                .execute("transfer", &json!({"accept":accept}))
                .unwrap();
            assert_eq!(
                outcome,
                if accept {
                    Outcome::Committed
                } else {
                    Outcome::Compensated
                }
            );
            assert_balances(&directory, terminal);
        }
    }
}

#[test]
fn instance_lock_and_protocol_version_fail_closed() {
    let directory = Directory::new();
    let first = directory.saga();
    assert!(
        DurableSaga::open(
            &directory.0.join("log.sqlite"),
            Ledger(directory.0.join("effects.sqlite"))
        )
        .is_err()
    );
    drop(first);
    let conn = rusqlite::Connection::open(directory.0.join("log.sqlite")).unwrap();
    conn.execute("INSERT INTO nirdosha_transact_log (txn_id,site_id,state,created_at,updated_at) VALUES ('old',-1,'pending',0,0)", []).unwrap();
    conn.execute(
        "INSERT INTO nirdosha_transact_context VALUES ('old','ledger-transfer/v0','{}')",
        [],
    )
    .unwrap();
    drop(conn);
    assert!(directory.saga().recover().is_err());
    assert!(!directory.0.join("effects.sqlite").exists());
}

#[test]
#[cfg(unix)]
fn process_stop_and_drop_terminate_the_group_and_release_resources() {
    use nirdosha_rt::native::WorkerProcess;
    for explicit_stop in [true, false] {
        let directory = Directory::new();
        let ready = directory.0.join("ready");
        let log = directory.0.join("process-log.sqlite");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "process_worker", "--nocapture"])
            .env("NIR_TEST_PROCESS_DIRECTORY", &directory.0)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let worker = WorkerProcess::spawn(&mut command).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "worker did not start");
        assert!(DurableSaga::open(&log, Ledger(directory.0.join("unused.sqlite"))).is_err());
        if explicit_stop {
            assert!(!worker.stop().unwrap().success());
        } else {
            drop(worker);
        }
        // The parent's heap is independent, and the worker's OS-owned lock
        // must be released even though its Rust destructors never ran.
        let reopened = DurableSaga::open(&log, Ledger(directory.0.join("unused.sqlite"))).unwrap();
        drop(reopened);
    }
}

#[test]
fn process_worker() {
    let Ok(path) = std::env::var("NIR_TEST_PROCESS_DIRECTORY") else {
        return;
    };
    let path = PathBuf::from(path);
    let _lock = DurableSaga::open(
        &path.join("process-log.sqlite"),
        Ledger(path.join("unused.sqlite")),
    )
    .unwrap();
    fs::write(path.join("ready"), "ready").unwrap();
    loop {
        std::thread::park();
    }
}
