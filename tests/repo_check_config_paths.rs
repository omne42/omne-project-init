use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn workspace_local_rejects_missing_root_primary_source_path() {
    let repo = init_repo(
        "config-root-primary-source",
        &["--project", "rust", "--layout", "root"],
    );

    replace_in_file(
        repo.path().join("repo-check.toml"),
        "primary_source_path = \"src/main.rs\"",
        "primary_source_path = \"src/missing.rs\"",
    );

    let error = run_generated_repo_check_fail(repo.path(), &["workspace", "local"]);
    assert!(
        error.contains("configured primary_source_path does not exist as a file"),
        "expected missing primary source path failure, got: {error}"
    );
}

#[test]
fn workspace_local_rejects_non_normalized_configured_paths() {
    let repo = init_repo(
        "config-non-normalized-path",
        &["--project", "rust", "--layout", "root"],
    );

    replace_in_file(
        repo.path().join("repo-check.toml"),
        "changelog_path = \"CHANGELOG.md\"",
        "changelog_path = \"./CHANGELOG.md\"",
    );

    let error = run_generated_repo_check_fail(repo.path(), &["workspace", "local"]);
    assert!(
        error.contains("changelog_path must be a normalized repository-relative path"),
        "expected normalized path failure, got: {error}"
    );
}

#[test]
fn workspace_local_rejects_crate_primary_source_outside_primary_crate() {
    let repo = init_repo(
        "config-crate-primary-source",
        &["--project", "rust", "--layout", "crate"],
    );

    replace_in_file(
        repo.path().join("repo-check.toml"),
        &format!(
            "primary_source_path = \"crates/{}/src/lib.rs\"",
            repo_slug(repo.path())
        ),
        "primary_source_path = \"README.md\"",
    );

    let error = run_generated_repo_check_fail(repo.path(), &["workspace", "local"]);
    assert!(
        error.contains(
            "primary_source_path `README.md` must live under the primary crate directory"
        ),
        "expected primary crate ownership failure, got: {error}"
    );
}

fn init_repo(prefix: &str, args: &[&str]) -> TempDir {
    let repo = TempDir::new(prefix);
    let mut cli_args = vec![
        "init".to_string(),
        repo.path().to_string_lossy().into_owned(),
    ];
    cli_args.extend(args.iter().map(|arg| (*arg).to_string()));
    cli_args.push("--no-git-init".to_string());
    run_cli(cli_args);
    repo
}

fn run_cli<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut command = Command::new(cli_binary());
    for arg in args {
        command.arg(arg.as_ref());
    }
    run_ok("omne-project-init", &mut command)
}

fn run_generated_repo_check_fail(repo_root: &Path, args: &[&str]) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = generated_repo_check_command(repo_root, args);
    run_fail("generated repo-check", &mut command)
}

fn generated_repo_check_command(repo_root: &Path, args: &[&str]) -> Command {
    let manifest_path = repo_root.join("tools/repo-check/Cargo.toml");
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--target-dir")
        .arg(generated_target_dir())
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--");

    let mut saw_repo_root = false;
    for arg in args {
        if *arg == "--repo-root" {
            saw_repo_root = true;
        }
        command.arg(arg);
    }
    if !saw_repo_root {
        command.arg("--repo-root").arg(repo_root);
    }
    command
}

fn generated_repo_check_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn generated_target_dir() -> PathBuf {
    static TARGET_ROOT: OnceLock<PathBuf> = OnceLock::new();
    TARGET_ROOT
        .get_or_init(|| {
            let path = env::temp_dir().join("omne-project-init-generated-target");
            fs::create_dir_all(&path).expect("failed to create generated target root");
            path
        })
        .clone()
}

fn replace_in_file(path: PathBuf, from: &str, to: &str) {
    let text = fs::read_to_string(&path).expect("read file for replacement");
    let updated = text.replace(from, to);
    assert_ne!(
        text,
        updated,
        "replacement target not found in {}",
        path.display()
    );
    fs::write(&path, updated).expect("write file after replacement");
}

fn run_ok(label: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: failed to execute command: {error}"));
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

fn run_fail(label: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: failed to execute command: {error}"));
    assert!(
        !output.status.success(),
        "{label} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        stderr
    }
}

fn cli_binary() -> &'static Path {
    static CLI_BINARY: OnceLock<PathBuf> = OnceLock::new();
    CLI_BINARY
        .get_or_init(|| PathBuf::from(env!("CARGO_BIN_EXE_omne-project-init")))
        .as_path()
}

fn repo_slug(repo_root: &Path) -> &str {
    repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .expect("temp repo path must end with a UTF-8 file name")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path =
            env::temp_dir().join(format!("repo-check-config-paths-{prefix}-{nanos}-{unique}"));
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
