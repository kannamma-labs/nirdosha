//! Emit separate per-rustc-invocation MIR certificates, never promote a
//! source-scan report to a stronger analysis mode. Publish only on success.
use crate::numeric::Report;
use nirdosha_contract_core::certificate::{Certificate, Mode, SourceFile, Subject, Tool};
use rustc_middle::ty::TyCtxt;
use rustc_span::{
    FileName,
    def_id::{LOCAL_CRATE, LocalDefId},
};
use serde_json::json;
use std::{collections::HashMap, path::PathBuf};

pub struct Pending {
    path: PathBuf,
    certificate: Certificate,
}
impl Pending {
    pub fn write(self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.path.parent().unwrap())?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.certificate)?)?;
        std::fs::rename(temporary, &self.path)?;
        eprintln!(
            "nirdosha: MIR numeric certificate ({}) → {}",
            nirdosha_smt_core::BACKEND,
            self.path.display()
        );
        Ok(())
    }
}
pub fn prepare(tcx: TyCtxt<'_>, reports: &HashMap<LocalDefId, Report>) -> std::io::Result<Pending> {
    let root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?)
        .canonicalize()?;
    let directory = std::env::var_os("NIRDOSHA_MIR_CERT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            tcx.sess
                .io
                .output_dir
                .clone()
                .unwrap_or_else(|| root.clone())
                .join("nirdosha/mir")
        });
    let mut files = Vec::new();
    let mut excluded = Vec::new();
    for file in tcx.sess.source_map().files().iter() {
        if file.is_imported() {
            continue;
        }
        if let FileName::Real(name) = &file.name {
            if let Some(path) = name.local_path() {
                let path = path.canonicalize()?;
                if path.starts_with(&root) {
                    let source = std::fs::read_to_string(&path)?;
                    if !file.src_hash.matches(&source) {
                        return Err(std::io::Error::other(format!(
                            "source changed during compilation: {}",
                            path.display()
                        )));
                    }
                    files.push(SourceFile::from_bytes(
                        path.strip_prefix(&root)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        source.as_bytes(),
                    ));
                } else {
                    excluded.push(path.display().to_string());
                }
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    excluded.sort();
    excluded.dedup();
    let mut reports: Vec<_> = reports
        .iter()
        .map(|(did, r)| (tcx.def_path_str(did.to_def_id()), r))
        .collect();
    reports.sort_by(|a, b| a.0.cmp(&b.0));
    let proofs = reports
        .iter()
        .flat_map(|(name, r)| r.proofs(name))
        .collect();
    let crate_name = tcx.crate_name(LOCAL_CRATE).to_string();
    let verification = json!({
        "profile":"nirdosha.mir-numeric/v1", "backend":nirdosha_smt_core::BACKEND,
        "overflow_checks":tcx.sess.overflow_checks(),
        "target":tcx.sess.opts.target_triple.to_string(),
        "arguments":std::env::args().skip(if std::env::var_os("NIRDOSHA_DRIVER").is_some(){2}else{1}).collect::<Vec<_>>(),
        "functions":reports.iter().map(|(name,r)|r.json(name)).collect::<Vec<_>>(),
        "sources_outside_package":excluded,
        "limitations":["per-assertion partial correctness, not whole-program panic freedom or termination",
            "normal paths only; loops and unsupported control flow yield no proofs for that body",
            "unknown calls and memory writes forget local facts; no numeric call summaries",
            "200ms/100000 Z3 resource units per query; 2048 block visits/256 depth per body",
            "elidable records eligibility only; this driver does not remove MIR assertions",
            "unsigned discharge records, not independently checkable solver proof objects or build provenance",
            "only listed local source files are hash-bound; external/generated inputs and dependencies are not covered"]
    });
    let cert = Certificate::new(
        Subject {
            package: std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| crate_name.clone()),
            version: std::env::var("CARGO_PKG_VERSION").ok(),
        },
        Tool {
            name: "nirdosha-driver".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            mode: Mode::MirDriver,
            toolchain: Some(env!("NIRDOSHA_RUSTC_VERSION").into()),
        },
        files,
        verification,
    )
    .with_proofs(proofs);
    let path = directory.join(format!(
        "contract-report-{crate_name}{}{}.json",
        tcx.sess.opts.cg.extra_filename,
        if tcx.sess.opts.test { "-test" } else { "" }
    ));
    Ok(Pending {
        path,
        certificate: cert,
    })
}
