use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn repo_root_cargo_test_no_run_isolated_from_parent_workspace() {
    let output = run_cargo(
        repo_root(),
        &["test", "--quiet", "--test", "docs_system", "--no-run"],
    );
    assert!(
        output.status.success(),
        "expected root cargo test --no-run to succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn template_repo_check_manifest_isolated_from_parent_workspace() {
    let output = run_cargo(
        repo_root(),
        &[
            "metadata",
            "--manifest-path",
            "templates/common/tools/repo-check/Cargo.toml",
            "--no-deps",
            "--format-version",
            "1",
        ],
    );
    assert!(
        output.status.success(),
        "expected template repo-check cargo metadata to succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"name\":\"omne-repo-check\""),
        "expected repo-check package metadata, got:\n{stdout}"
    );
}

fn run_cargo(repo_root: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new("cargo");
    command.args(args).current_dir(repo_root);
    command.env("CARGO_TARGET_DIR", fresh_target_dir("workspace-isolation"));
    command.env("CARGO_TERM_COLOR", "never");
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to execute cargo {:?}: {error}", args))
}

fn fresh_target_dir(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    env::temp_dir().join(format!("omne-project-init-{prefix}-{nanos}-{unique}"))
}
