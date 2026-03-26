use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn manifest_lists_expected_files_for_supported_projects() {
    let sandbox = TempDir::new("manifest-sandbox");

    let rust_output = run_cli([
        "manifest",
        sandbox.path().join("rust-crate").to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
    ]);
    assert_manifest_contains(
        &rust_output,
        &[
            "\"githooks/pre-commit\"",
            "\"repo-check.toml\"",
            "\"tools/repo-check/src/main.rs\"",
            "\"crates/rust-crate/Cargo.toml\"",
            "\"crates/rust-crate/CHANGELOG.md\"",
        ],
    );

    let python_output = run_cli([
        "manifest",
        sandbox.path().join("python-app").to_string_lossy().as_ref(),
        "--project",
        "python",
    ]);
    assert_manifest_contains(
        &python_output,
        &[
            "\"pyproject.toml\"",
            "\"python_app/__init__.py\"",
            "\"CHANGELOG.md\"",
            "\"repo-check.toml\"",
        ],
    );

    let node_output = run_cli([
        "manifest",
        sandbox.path().join("node-app").to_string_lossy().as_ref(),
        "--project",
        "nodejs",
    ]);
    assert_manifest_contains(
        &node_output,
        &[
            "\"package.json\"",
            "\"src/index.js\"",
            "\"test/basic.test.js\"",
            "\"tools/repo-check/Cargo.toml\"",
        ],
    );
}

#[test]
fn init_writes_expected_metadata_for_rust_layouts() {
    let rust_crate = init_repo(
        "rust-crate-layout",
        &["--project", "rust", "--layout", "crate"],
    );
    let rust_root = init_repo(
        "rust-root-layout",
        &["--project", "rust", "--layout", "root"],
    );
    let rust_crate_slug = repo_slug(rust_crate.path());

    let crate_repo_check = fs::read_to_string(rust_crate.path().join("repo-check.toml"))
        .expect("failed to read crate repo-check.toml");
    assert!(crate_repo_check.contains("layout = \"crate\""));
    assert!(crate_repo_check.contains(&format!(
        "package_manifest_path = \"crates/{rust_crate_slug}/Cargo.toml\""
    )));
    assert!(crate_repo_check.contains(&format!(
        "changelog_path = \"crates/{rust_crate_slug}/CHANGELOG.md\""
    )));

    let crate_workspace = fs::read_to_string(rust_crate.path().join("Cargo.toml"))
        .expect("failed to read crate workspace Cargo.toml");
    assert!(crate_workspace.contains("exclude = [\"tools/repo-check\"]"));
    assert!(crate_workspace.contains("resolver = \"3\""));

    let crate_manifest = fs::read_to_string(
        rust_crate
            .path()
            .join("crates")
            .join(rust_crate_slug)
            .join("Cargo.toml"),
    )
    .expect("failed to read crate package Cargo.toml");
    assert!(crate_manifest.contains("edition = \"2024\""));
    assert!(crate_manifest.contains("version.workspace = true"));

    let root_repo_check = fs::read_to_string(rust_root.path().join("repo-check.toml"))
        .expect("failed to read root repo-check.toml");
    assert!(root_repo_check.contains("layout = \"root\""));
    assert!(root_repo_check.contains("package_manifest_path = \"Cargo.toml\""));
    assert!(root_repo_check.contains("changelog_path = \"CHANGELOG.md\""));

    let root_manifest = fs::read_to_string(rust_root.path().join("Cargo.toml"))
        .expect("failed to read root Cargo.toml");
    assert!(root_manifest.contains("edition = \"2024\""));
    assert!(root_manifest.contains("exclude = [\"tools/repo-check\"]"));

    let repo_check_manifest =
        fs::read_to_string(rust_root.path().join("tools/repo-check/Cargo.toml"))
            .expect("failed to read generated repo-check Cargo.toml");
    assert!(repo_check_manifest.contains("edition = \"2024\""));
}

#[test]
fn generated_rust_repo_check_workspace_local_passes_for_root_and_crate() {
    let rust_crate = init_repo(
        "rust-crate-smoke",
        &["--project", "rust", "--layout", "crate"],
    );
    let rust_root = init_repo(
        "rust-root-smoke",
        &["--project", "rust", "--layout", "root"],
    );

    run_generated_repo_check(rust_crate.path(), &["workspace", "local"]);
    run_generated_repo_check(rust_root.path(), &["workspace", "local"]);
}

#[test]
fn generated_python_repo_check_workspace_local_passes_when_python_is_available() {
    if !has_python() {
        eprintln!("skipping python smoke test: no supported Python interpreter found");
        return;
    }

    let repo = init_repo("python-smoke", &["--project", "python"]);
    run_generated_repo_check(repo.path(), &["workspace", "local"]);
}

#[test]
fn generated_node_repo_check_workspace_local_passes_when_node_is_available() {
    if !command_works("node", &["--version"]) {
        eprintln!("skipping node smoke test: `node` not found");
        return;
    }

    let repo = init_repo("node-smoke", &["--project", "nodejs"]);
    run_generated_repo_check(repo.path(), &["workspace", "local"]);
}

#[test]
fn generated_rust_repo_check_git_flow_passes() {
    if !command_works("git", &["--version"]) {
        eprintln!("skipping git flow smoke test: `git` not found");
        return;
    }

    let repo = init_repo("rust-git-flow", &["--project", "rust", "--layout", "crate"]);
    git_init(repo.path());
    run_ok(
        "git checkout feature branch",
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("checkout")
            .arg("-b")
            .arg("feat/smoke-check"),
    );
    run_generated_repo_check(repo.path(), &["install-hooks"]);

    let hooks_path = run_ok(
        "git config core.hooksPath",
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("config")
            .arg("--get")
            .arg("core.hooksPath"),
    );
    assert_eq!(hooks_path.trim(), "githooks");

    run_ok(
        "git add",
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("add")
            .arg("."),
    );

    run_generated_repo_check(repo.path(), &["pre-commit"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.test");
    fs::write(&commit_msg, "feat(repo): initial scaffold\n")
        .expect("failed to write commit message file");
    run_generated_repo_check(
        repo.path(),
        &[
            "commit-msg",
            "--commit-msg-file",
            commit_msg.to_string_lossy().as_ref(),
        ],
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

fn run_generated_repo_check(repo_root: &Path, args: &[&str]) -> String {
    let manifest_path = repo_root.join("tools/repo-check/Cargo.toml");
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--target-dir")
        .arg(shared_generated_target_dir())
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

    run_ok("generated repo-check", &mut command)
}

fn git_init(repo_root: &Path) {
    let init_with_branch = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("init")
        .arg("-b")
        .arg("main")
        .output()
        .expect("failed to execute `git init -b main`");

    if init_with_branch.status.success() {
        return;
    }

    run_ok(
        "git init",
        Command::new("git").arg("-C").arg(repo_root).arg("init"),
    );
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("branch")
        .arg("-m")
        .arg("main")
        .output()
        .expect("failed to execute `git branch -m main`");
}

fn assert_manifest_contains(output: &str, expected_entries: &[&str]) {
    for entry in expected_entries {
        assert!(
            output.contains(entry),
            "manifest output did not contain {entry}\n\noutput:\n{output}"
        );
    }
}

fn run_ok(label: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: failed to execute command: {error}"));
    if output.status.success() {
        return String::from_utf8_lossy(&output.stdout).into_owned();
    }

    panic!(
        "{label}: command failed\nstatus: {}\n{}",
        output.status,
        render_output(&output)
    );
}

fn render_output(output: &Output) -> String {
    format!(
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn cli_binary() -> &'static str {
    env!("CARGO_BIN_EXE_omne-project-init")
}

fn repo_slug(repo_root: &Path) -> &str {
    repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .expect("temp repo path must end with a UTF-8 file name")
}

fn shared_generated_target_dir() -> &'static Path {
    static TARGET_DIR: OnceLock<PathBuf> = OnceLock::new();
    TARGET_DIR.get_or_init(|| {
        let path = env::temp_dir().join("omne-project-init-generated-target");
        fs::create_dir_all(&path).expect("failed to create shared generated target dir");
        path
    })
}

fn has_python() -> bool {
    command_works("python", &["--version"])
        || command_works("python3", &["--version"])
        || command_works("py", &["-3", "--version"])
}

fn command_works(program: &str, args: &[&str]) -> bool {
    matches!(
        Command::new(program).args(args).output(),
        Ok(output) if output.status.success()
    )
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        loop {
            let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is before UNIX_EPOCH")
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "omne-project-init-{prefix}-{pid}-{nanos}-{counter}",
                pid = std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create temp dir {}: {error}", path.display()),
            }
        }
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
