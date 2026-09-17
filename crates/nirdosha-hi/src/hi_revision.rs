//! `hi`-owned revision management, via real `git` (2026-09-16, the
//! first pass -- "to start with," per the user's own framing). `hi`
//! already owns the project's data structure (the knowledge graph,
//! `hi_graph.rs`) and content (ingested documents, `hi_graph::
//! ingest_document`); this is the third leg, revision history, kept as
//! plain `git` commits in the project's own repository rather than a
//! bespoke versioning scheme -- the project root's `.git` is auto-
//! `git init`'d if it doesn't already exist.
//!
//! Read-only for now: `log`/`diff` only. A `revert`/`checkout`-style
//! mutation is real, separate danger (overwriting an uncommitted
//! generated draft) and deserves its own confirmation step later, not
//! bundled into this first pass.

use std::path::Path;
use std::process::Command;

/// Paths this module ever `git add`s or reads history for -- never the
/// whole project root, so a user's own unrelated files (a compiled
/// binary sitting next to `.nir/`, say) are never swept into a commit
/// `hi` didn't ask about.
fn tracked_paths(root: &Path) -> Vec<std::path::PathBuf> {
    vec![root.join(".nir")]
}

fn run_git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| format!("failed to invoke git {args:?} in {}: {e}", root.display()))
}

fn git_ok(output: &std::process::Output) -> bool {
    output.status.success()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The default `.gitignore` lines a fresh project needs so `hi`'s own
/// build artifacts never get committed -- appended, never overwritten:
/// a project that already has a `.gitignore` (or adds its own lines to
/// the one this seeds) keeps everything it already had.
const DEFAULT_GITIGNORE_LINES: &[&str] = &[
    "/target/",
    "/.nir/generated/hi_build",
    "/.nir/generated/hi_build.certificate.json",
    "/.nir/generated/hi_preview",
    "/.nir/generated/v2-build/",
    "/.nir/generated/attempts/",
];

/// `git init`s `root` if it isn't already a repository, and seeds/
/// extends its `.gitignore` with [`DEFAULT_GITIGNORE_LINES`] (only the
/// lines not already present, in whatever order/casing they already
/// appear -- a plain substring-of-lines check, not a full gitignore-
/// pattern-equivalence one, same "good enough, disclosed" scope this
/// crate uses elsewhere). Idempotent: safe to call before every
/// `:generate`/`:publish`.
pub fn ensure_repo(root: &Path) -> Result<(), String> {
    if !root.join(".git").exists() {
        let output = run_git(root, &["init"])?;
        if !git_ok(&output) {
            return Err(format!("git init failed in {}: {}", root.display(), stderr_of(&output)));
        }
    }
    let gitignore_path = root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    let missing: Vec<&&str> = DEFAULT_GITIGNORE_LINES.iter().filter(|line| !existing.lines().any(|l| l.trim() == **line)).collect();
    if !missing.is_empty() {
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        for line in missing {
            updated.push_str(line);
            updated.push('\n');
        }
        std::fs::write(&gitignore_path, updated).map_err(|e| format!("writing {}: {e}", gitignore_path.display()))?;
    }
    Ok(())
}

/// Stages [`tracked_paths`] and commits, only if something is actually
/// staged (`Ok(None)` for "nothing to commit" rather than an error --
/// a `:generate` that reproduces byte-identical output, or a `:publish`
/// with nothing new since the last one, is a normal, silent no-op, not
/// a failure). Returns the new commit's hash on a real commit.
pub fn commit_revision(root: &Path, message: &str) -> Result<Option<String>, String> {
    ensure_repo(root)?;
    let paths = tracked_paths(root);
    let mut add_args: Vec<&str> = vec!["add", "-A", "--"];
    let path_strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    add_args.extend(path_strs.iter().map(|s| s.as_str()));
    let add_output = run_git(root, &add_args)?;
    if !git_ok(&add_output) {
        return Err(format!("git add failed in {}: {}", root.display(), stderr_of(&add_output)));
    }

    let diff_check = run_git(root, &["diff", "--cached", "--quiet"])?;
    if diff_check.status.success() {
        return Ok(None); // nothing staged -- a real, silent no-op
    }

    let commit_output = run_git(root, &["commit", "-m", message])?;
    if !git_ok(&commit_output) {
        return Err(format!("git commit failed in {}: {}", root.display(), stderr_of(&commit_output)));
    }
    let rev_output = run_git(root, &["rev-parse", "HEAD"])?;
    if !git_ok(&rev_output) {
        return Err(format!("git rev-parse HEAD failed in {}: {}", root.display(), stderr_of(&rev_output)));
    }
    Ok(Some(String::from_utf8_lossy(&rev_output.stdout).trim().to_string()))
}

#[derive(serde::Serialize)]
pub struct RevisionEntry {
    pub hash: String,
    pub message: String,
    pub timestamp: String,
}

/// The last `limit` commits touching [`tracked_paths`], newest first --
/// `Ok(vec![])` (not an error) when `root` isn't a git repository at
/// all, or has no commits yet, so `GET /api/git?op=log` can report an
/// empty, honest history rather than a 500.
pub fn log(root: &Path, limit: usize) -> Result<Vec<RevisionEntry>, String> {
    if !root.join(".git").exists() {
        return Ok(Vec::new());
    }
    let paths = tracked_paths(root);
    let path_strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let mut args: Vec<&str> = vec!["log", "--format=%H%x1f%s%x1f%cI", "-n"];
    let limit_str = limit.to_string();
    args.push(&limit_str);
    args.push("--");
    args.extend(path_strs.iter().map(|s| s.as_str()));
    let output = run_git(root, &args)?;
    if !git_ok(&output) {
        // An empty repo (no commits yet) exits non-zero here -- treated
        // as "no history," not an error, same reasoning as the missing
        // `.git` case above.
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\u{1f}');
            let hash = parts.next()?.to_string();
            let message = parts.next()?.to_string();
            let timestamp = parts.next()?.to_string();
            Some(RevisionEntry { hash, message, timestamp })
        })
        .collect())
}

/// `git show <rev>` scoped to [`tracked_paths`] -- a unified diff of
/// exactly what that revision changed under `.nir/`, nothing from
/// elsewhere in the project even if `<rev>` also touched other files
/// (which `commit_revision` never lets happen, but a hand-made commit
/// in the same repo could).
pub fn diff(root: &Path, rev: &str) -> Result<String, String> {
    let paths = tracked_paths(root);
    let path_strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let mut args: Vec<&str> = vec!["show", rev, "--"];
    args.extend(path_strs.iter().map(|s| s.as_str()));
    let output = run_git(root, &args)?;
    if !git_ok(&output) {
        return Err(format!("git show {rev} failed in {}: {}", root.display(), stderr_of(&output)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("nirdosha_hi_revision_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn ensure_repo_inits_and_seeds_gitignore_once() {
        let dir = scratch_dir("ensure_repo");
        ensure_repo(&dir).expect("first call inits");
        assert!(dir.join(".git").exists());
        let gitignore = std::fs::read_to_string(dir.join(".gitignore")).expect("gitignore written");
        assert!(gitignore.contains("/target/"));
        ensure_repo(&dir).expect("second call is a no-op, not an error");
    }

    #[test]
    fn commit_revision_is_a_no_op_when_nothing_changed() {
        let dir = scratch_dir("commit_noop");
        ensure_repo(&dir).expect("init");
        let result = commit_revision(&dir, "nothing to see here").expect("must not error");
        assert!(result.is_none(), "an empty .nir/ has nothing to stage");
    }

    #[test]
    fn commit_revision_commits_and_log_reports_it() {
        let dir = scratch_dir("commit_and_log");
        ensure_repo(&dir).expect("init");
        std::fs::create_dir_all(dir.join(".nir/generated")).expect("mkdir");
        std::fs::write(dir.join(".nir/generated/hi_build.nir"), "fn main() {}\n").expect("write");

        let hash = commit_revision(&dir, "generate: 1 unit(s)").expect("commit must succeed").expect("something was staged");
        assert_eq!(hash.len(), 40, "a full git SHA-1 hash");

        let entries = log(&dir, 10).expect("log must succeed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hash, hash);
        assert_eq!(entries[0].message, "generate: 1 unit(s)");

        let text = diff(&dir, &hash).expect("diff must succeed");
        assert!(text.contains("hi_build.nir"), "got:\n{text}");
    }

    #[test]
    fn log_is_empty_not_an_error_when_root_is_not_a_repo() {
        let dir = scratch_dir("log_no_repo");
        assert!(log(&dir, 10).expect("must not error").is_empty());
    }
}
