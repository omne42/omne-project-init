use std::env;
use std::fs;
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
fn repo_root_manifest_stays_isolated_inside_unrelated_parent_workspace() {
    let sandbox = SyntheticParentWorkspace::new("repo-root");
    sandbox.write_child_manifest(
        "nested/omne-project-init",
        &fs::read_to_string(repo_root().join("Cargo.toml")).expect("read root Cargo.toml"),
    );

    let output = run_cargo(
        sandbox.root(),
        &[
            "metadata",
            "--manifest-path",
            "nested/omne-project-init/Cargo.toml",
            "--no-deps",
            "--format-version",
            "1",
        ],
    );
    assert!(
        output.status.success(),
        "expected copied root manifest to stay isolated inside parent workspace\nstdout:\n{}\nstderr:\n{}",
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

#[test]
fn copied_template_repo_check_manifest_stays_isolated_inside_unrelated_parent_workspace() {
    let sandbox = SyntheticParentWorkspace::new("repo-check");
    sandbox.write_child_manifest(
        "nested/tools/repo-check",
        &fs::read_to_string(repo_root().join("templates/common/tools/repo-check/Cargo.toml"))
            .expect("read template repo-check Cargo.toml"),
    );

    let output = run_cargo(
        sandbox.root(),
        &[
            "metadata",
            "--manifest-path",
            "nested/tools/repo-check/Cargo.toml",
            "--no-deps",
            "--format-version",
            "1",
        ],
    );
    assert!(
        output.status.success(),
        "expected copied repo-check manifest to stay isolated inside parent workspace\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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

struct SyntheticParentWorkspace {
    root: PathBuf,
}

impl SyntheticParentWorkspace {
    fn new(prefix: &str) -> Self {
        let root = fresh_target_dir(prefix);
        fs::create_dir_all(&root).expect("create synthetic workspace root");
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"workspace-member\"]\n")
            .expect("write synthetic workspace manifest");
        fs::create_dir_all(root.join("workspace-member/src"))
            .expect("create synthetic workspace member");
        fs::write(
            root.join("workspace-member/Cargo.toml"),
            "[package]\nname = \"workspace-member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write synthetic workspace member manifest");
        fs::write(root.join("workspace-member/src/lib.rs"), "pub fn workspace_member() {}\n")
            .expect("write synthetic workspace member source");
        Self { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn write_child_manifest(&self, relative_dir: &str, manifest: &str) {
        let child_dir = self.root.join(relative_dir);
        fs::create_dir_all(child_dir.join("src")).expect("create synthetic child source dir");
        fs::write(child_dir.join("Cargo.toml"), manifest).expect("write synthetic child manifest");
        fs::write(child_dir.join("src/main.rs"), "fn main() {}\n")
            .expect("write synthetic child main.rs");
    }
}

impl Drop for SyntheticParentWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
