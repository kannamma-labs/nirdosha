//! The v2 (Rust + `nirdosha:*` comment layer) analogue of
//! `capabilities.rs`: one small, self-contained program per major v2
//! construct, checked against the real two-reader pipeline
//! (`v2_verify::verify_v2_source` — `cargo build` + `cargo-nirdosha`'s
//! in-process scanner) rather than a hand-typed claim. Mirrors the
//! same construct list `capabilities.rs` checks for native `.nir`, so
//! `get_nirdosha_constructs` teaches the v2 shape of exactly the
//! constructs an agent already expects to find there.

use crate::v2_verify::{V2Verdict, verify_v2_source};

pub struct V2Capability {
    pub name: &'static str,
    pub source: &'static str,
}

pub struct V2CapabilityResult {
    pub name: &'static str,
    pub passed: bool,
    pub diagnostic: Option<String>,
    pub source: &'static str,
}

pub fn v2_capabilities() -> Vec<V2Capability> {
    vec![
        V2Capability {
            name: "fn + arithmetic",
            source: "fn add(a: i64, b: i64) -> i64 {\n    a + b\n}\n\nfn main() {\n    println!(\"{}\", add(2, 3));\n}\n",
        },
        V2Capability {
            name: "struct + field access",
            source: "struct Point {\n    x: i64,\n    y: i64,\n}\n\nfn main() {\n    let p = Point { x: 1, y: 2 };\n    println!(\"{}\", p.x);\n}\n",
        },
        V2Capability {
            name: "enum + match",
            source: "enum Status {\n    Pending,\n    Approved,\n    Rejected(String),\n}\n\nfn describe(s: Status) -> Status {\n    match s {\n        Status::Pending => Status::Approved,\n        Status::Approved => Status::Approved,\n        Status::Rejected(reason) => Status::Rejected(reason),\n    }\n}\n\nfn main() {\n    let _s = describe(Status::Pending);\n    println!(\"described\");\n}\n",
        },
        V2Capability {
            name: "validate (Hoare) contract",
            source: "/// nirdosha:validate {\"fn\":\"charge_cents\",\"pre\":[\"amount_cents >= 0 && amount_cents <= balance_cents\"],\"post\":[\"result >= 0\"]}\nfn charge_cents(amount_cents: i64, balance_cents: i64) -> i64 {\n    balance_cents - amount_cents\n}\n\nfn main() {\n    println!(\"{}\", charge_cents(10, 100));\n}\n",
        },
        V2Capability {
            name: "workflow (state machine)",
            source: "/// nirdosha:workflow {\"name\":\"Approval\",\"data\":{\"struct\":\"ApprovalData\",\"fields\":{\"amount\":\"i64\"}},\"states\":{\"Pending\":{\"transitions\":{\"Approve\":\"Approved\",\"Reject\":\"Rejected\"}},\"Approved\":{\"terminal\":true},\"Rejected\":{\"terminal\":true}}}\nstruct ApprovalData {\n    amount: i64,\n}\n\nenum ApprovalEvent {\n    Approve,\n    Reject,\n}\n\nuse std::collections::HashMap;\nuse std::sync::Mutex;\n\nstatic STATES: Mutex<Option<HashMap<i64, String>>> = Mutex::new(None);\n\nfn start_approval(data: ApprovalData) -> i64 {\n    let id = data.amount;\n    let mut guard = STATES.lock().unwrap();\n    guard.get_or_insert_with(HashMap::new).insert(id, \"Pending\".to_string());\n    id\n}\n\nfn advance_approval(id: i64, ev: ApprovalEvent) -> bool {\n    let mut guard = STATES.lock().unwrap();\n    let Some(map) = guard.as_mut() else { return false };\n    let Some(state) = map.get(&id) else { return false };\n    let next = match (state.as_str(), &ev) {\n        (\"Pending\", ApprovalEvent::Approve) => Some(\"Approved\"),\n        (\"Pending\", ApprovalEvent::Reject) => Some(\"Rejected\"),\n        _ => None,\n    };\n    match next {\n        Some(n) => {\n            map.insert(id, n.to_string());\n            true\n        }\n        None => false,\n    }\n}\n\nfn main() {\n    let id = start_approval(ApprovalData { amount: 250000 });\n    let ok = advance_approval(id, ApprovalEvent::Approve);\n    println!(\"advanced {ok}\");\n}\n",
        },
        V2Capability {
            name: "transact (verify/compensate claim)",
            source: "/// nirdosha:transact {\"verify\":\"check_amount\",\"compensate\":\"refund_amount\"}\nfn charge(amount: i64) -> bool {\n    if check_amount(amount) {\n        commit_amount(amount);\n        true\n    } else {\n        refund_amount(amount);\n        false\n    }\n}\n\nfn check_amount(amount: i64) -> bool {\n    amount > 0\n}\nfn commit_amount(_amount: i64) {}\nfn refund_amount(_amount: i64) {}\n\nfn main() {\n    println!(\"{}\", charge(10));\n}\n",
        },
        V2Capability {
            name: "screen + serve",
            source: "/// nirdosha:screen {\"for\":\"PaymentRequest\",\"title\":\"Payments\",\"fields\":{\"amount_cents\":{\"label\":\"Amount\"}}}\nstruct PaymentRequest {\n    request_id: i64,\n    amount_cents: i64,\n    status: String,\n}\n\nfn list_payment_request() -> Vec<PaymentRequest> {\n    Vec::new()\n}\n\nfn stat_open_approval_count() -> i64 {\n    0\n}\n\n/// nirdosha:serve {\"routes\":[\"list_payment_request\",\"stat_open_approval_count\"]}\nfn serve_ready() -> bool {\n    true\n}\n\nfn main() {\n    println!(\"ready\");\n}\n",
        },
        V2Capability {
            name: "json (serde_json)",
            source: "fn main() {\n    let doc: serde_json::Value = serde_json::json!({\"user\": \"ravi\"});\n    println!(\"{}\", doc[\"user\"]);\n}\n",
        },
        V2Capability {
            name: "identity + acquire (RoleProof)",
            source: "nirdosha_rt::roles! {\n    FinanceDirector = \"finance_director\";\n}\n\n#[nirdosha_rt::contract(requires(role = \"finance_director\"))]\nfn try_approve(amount: i64) -> bool {\n    amount > 0\n}\n\nfn main() {\n    let auth = nirdosha_rt::Auth::login(\"meera\", &[\"finance_director\"]);\n    let ok = match auth.prove::<nirdosha_roles::FinanceDirector>() {\n        Ok(proof) => try_approve(&proof, 1),\n        Err(_) => false,\n    };\n    println!(\"approved {ok}\");\n}\n",
        },
    ]
}

fn verdict_diagnostic(name: &str, verdict: &Result<V2Verdict, String>) -> (bool, Option<String>) {
    match verdict {
        Ok(v) if v.passed() => (true, None),
        Ok(v) => {
            let mut msg = String::new();
            if !v.builds {
                msg.push_str(
                    v.build_diagnostic
                        .as_deref()
                        .unwrap_or("cargo build failed"),
                );
            }
            if !v.violations.is_empty() {
                if !msg.is_empty() {
                    msg.push('\n');
                }
                msg.push_str(&v.violations.join("\n"));
            }
            (false, Some(msg))
        }
        Err(e) => (false, Some(format!("{name}: {e}"))),
    }
}

/// Runs every `v2_capabilities()` entry against the real two-reader
/// pipeline and reports pass/fail per construct.
pub fn run_v2_capability_checks() -> Vec<V2CapabilityResult> {
    v2_capabilities()
        .into_iter()
        .map(|cap| {
            let verdict = verify_v2_source(cap.source);
            let (passed, diagnostic) = verdict_diagnostic(cap.name, &verdict);
            V2CapabilityResult {
                name: cap.name,
                passed,
                diagnostic,
                source: cap.source,
            }
        })
        .collect()
}
