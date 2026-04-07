use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn repository_root_cargo_metadata_is_workspace_isolated() {
    let output = run_ok(
        "cargo metadata",
        Command::new("cargo")
            .arg("metadata")
            .arg("--manifest-path")
            .arg(repo_root().join("Cargo.toml"))
            .arg("--no-deps")
            .arg("--format-version")
            .arg("1"),
    );
    assert!(
        output.contains("\"name\":\"omne-project-init\""),
        "unexpected cargo metadata output:\n{output}"
    );
}

#[test]
fn generated_repo_check_manifest_is_workspace_isolated_for_python_and_node() {
    let python = init_repo("python-workspace-boundary", &["--project", "python"]);
    let node = init_repo("node-workspace-boundary", &["--project", "nodejs"]);

    assert_repo_check_metadata_succeeds(python.path());
    assert_repo_check_metadata_succeeds(node.path());
}

#[test]
fn generated_repo_check_manifest_joins_rust_workspace_without_parent_leakage() {
    let rust_root = init_repo(
        "rust-root-workspace-boundary",
        &["--project", "rust", "--layout", "root"],
    );
    let rust_crate = init_repo(
        "rust-crate-workspace-boundary",
        &["--project", "rust", "--layout", "crate"],
    );

    assert_workspace_metadata_succeeds(rust_root.path());
    assert_workspace_metadata_succeeds(rust_crate.path());
    assert_repo_check_metadata_succeeds(rust_root.path());
    assert_repo_check_metadata_succeeds(rust_crate.path());
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn init_repo(prefix: &str, args: &[&str]) -> TempDir {
    let repo = TempDir::new(prefix);
    let mut command = Command::new(cli_binary());
    command
        .arg("init")
        .arg(repo.path())
        .args(args)
        .arg("--no-git-init");
    run_ok("omne-project-init", &mut command);
    repo
}

fn assert_workspace_metadata_succeeds(repo_root: &Path) {
    let output = run_ok(
        "workspace cargo metadata",
        Command::new("cargo")
            .current_dir(repo_root)
            .arg("metadata")
            .arg("--manifest-path")
            .arg("Cargo.toml")
            .arg("--no-deps")
            .arg("--format-version")
            .arg("1"),
    );
    assert!(
        output.contains("/tools/repo-check/Cargo.toml"),
        "workspace metadata did not include repo-check:\n{output}"
    );
}

fn assert_repo_check_metadata_succeeds(repo_root: &Path) {
    let output = run_ok(
        "repo-check cargo metadata",
        Command::new("cargo")
            .current_dir(repo_root)
            .arg("metadata")
            .arg("--manifest-path")
            .arg("tools/repo-check/Cargo.toml")
            .arg("--no-deps")
            .arg("--format-version")
            .arg("1"),
    );
    assert!(
        output.contains("/tools/repo-check/Cargo.toml"),
        "repo-check metadata did not resolve the generated manifest:\n{output}"
    );
}

fn cli_binary() -> &'static str {
    env!("CARGO_BIN_EXE_omne-project-init")
}

fn run_ok(label: &str, command: &mut Command) -> String {
    let rendered = format!("{command:?}");
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label} failed to start: {error}\ncommand: {rendered}"));
    if output.status.success() {
        return String::from_utf8_lossy(&output.stdout).into_owned();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    panic!(
        "{label} failed\ncommand: {rendered}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status, stdout, stderr
    );
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time before UNIX_EPOCH")
            .as_nanos();
        let path = env::temp_dir().join(format!("omne-project-init-{prefix}-{timestamp}-{unique}"));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);
