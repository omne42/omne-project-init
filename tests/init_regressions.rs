use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn digit_prefixed_repo_names_generate_valid_rust_and_python_scaffolds() {
    let sandbox = TempDir::new("digit-prefix");

    let rust_repo = sandbox.path().join("123app");
    run_cli([
        "init",
        rust_repo.to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
        "--no-setup-hooks",
    ]);
    let rust_manifest = rust_repo
        .join("crates")
        .join("app-123app")
        .join("Cargo.toml");
    assert!(
        rust_manifest.is_file(),
        "expected derived rust crate manifest at {}",
        rust_manifest.display()
    );
    assert!(
        fs::read_to_string(&rust_manifest)
            .expect("read derived rust manifest")
            .contains("name = \"app-123app\""),
        "expected derived rust package name to be stabilized"
    );
    run_ok(
        "generated rust cargo check",
        Command::new("cargo")
            .arg("check")
            .arg("--workspace")
            .arg("--all-targets")
            .arg("--all-features")
            .arg("--target-dir")
            .arg(shared_generated_target_dir())
            .current_dir(&rust_repo),
    );

    let python_repo = sandbox.path().join("123py");
    run_cli([
        "init",
        python_repo.to_string_lossy().as_ref(),
        "--project",
        "python",
        "--no-git-init",
        "--no-setup-hooks",
    ]);
    let python_package = python_repo.join("pkg_123py").join("__init__.py");
    assert!(
        python_package.is_file(),
        "expected derived python import package at {}",
        python_package.display()
    );
    assert!(
        !python_repo.join("123py").exists(),
        "unexpected invalid python import package directory created"
    );

    if let Some(python) = detect_python_command() {
        let mut command = Command::new(python[0]);
        command.args(&python[1..]);
        command
            .arg("-m")
            .arg("compileall")
            .arg("pkg_123py")
            .current_dir(&python_repo);
        run_ok("generated python compileall", &mut command);
    }
}

#[test]
fn force_regeneration_removes_old_scaffold_artifacts_but_keeps_git_dir() {
    let sandbox = TempDir::new("force-cleanup");
    let repo = sandbox.path().join("force-cleanup");

    run_cli([
        "init",
        repo.to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
        "--no-setup-hooks",
    ]);
    fs::create_dir_all(repo.join(".git")).expect("create .git marker");

    run_cli([
        "init",
        repo.to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
        "--no-setup-hooks",
    ]);

    assert!(repo.join(".git").is_dir(), "expected .git to be preserved");
    assert!(repo.join("pyproject.toml").is_file());
    assert!(repo.join("force_cleanup").join("__init__.py").is_file());
    assert!(
        !repo.join("Cargo.toml").exists(),
        "old rust root manifest should be removed"
    );
    assert!(
        !repo.join("crates").exists(),
        "old rust crate layout should be removed"
    );
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

fn shared_generated_target_dir() -> &'static Path {
    static TARGET_DIR: OnceLock<PathBuf> = OnceLock::new();
    TARGET_DIR.get_or_init(|| {
        let path = env::temp_dir().join("omne-project-init-init-regressions-target");
        fs::create_dir_all(&path).expect("failed to create shared generated target dir");
        path
    })
}

fn detect_python_command() -> Option<Vec<&'static str>> {
    [vec!["python"], vec!["python3"], vec!["py", "-3"]]
        .into_iter()
        .find(|candidate| command_works(candidate[0], &candidate[1..], &["--version"]))
}

fn command_works(program: &str, prefix_args: &[&str], probe_args: &[&str]) -> bool {
    matches!(
        Command::new(program)
            .args(prefix_args)
            .args(probe_args)
            .output(),
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
