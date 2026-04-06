use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn force_reinit_replaces_previous_scaffold_without_residue() {
    let _guard = template_fixture_lock()
        .lock()
        .expect("template fixture lock poisoned");
    let repo = TempDir::new("force-reinit");
    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
    ]);

    assert!(repo.path().join("Cargo.toml").is_file());
    assert!(repo.path().join("crates").is_dir());

    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
    ]);

    assert!(repo.path().join("pyproject.toml").is_file());
    assert!(repo.path().join("CHANGELOG.md").is_file());
    assert!(!repo.path().join("Cargo.toml").exists());
    assert!(!repo.path().join("crates").exists());

    let repo_check =
        fs::read_to_string(repo.path().join("repo-check.toml")).expect("read repo-check.toml");
    assert!(repo_check.contains("project_kind = \"python\""));
    assert!(repo_check.contains("layout = \"root\""));
    assert!(repo_check.contains("package_manifest_path = \"pyproject.toml\""));
    assert!(repo_check.contains("changelog_path = \"CHANGELOG.md\""));
}

#[test]
fn project_templates_override_common_files_with_same_rendered_path() {
    let _guard = template_fixture_lock()
        .lock()
        .expect("template fixture lock poisoned");
    let common_override = repo_template_root()
        .join("common")
        .join("override-sentinel.txt");
    let project_override = repo_template_root()
        .join("projects")
        .join("python")
        .join("root")
        .join("override-sentinel.txt");
    let _common_cleanup = PathCleanup::new(common_override.clone());
    let _project_cleanup = PathCleanup::new(project_override.clone());

    fs::write(&common_override, "common layer\n").expect("write common override fixture");
    fs::write(&project_override, "python layer\n").expect("write project override fixture");

    let repo = TempDir::new("template-override");
    let manifest = run_cli([
        "manifest",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
    ]);
    assert_eq!(manifest.matches("\"override-sentinel.txt\"").count(), 1);

    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--no-git-init",
    ]);
    let rendered =
        fs::read_to_string(repo.path().join("override-sentinel.txt")).expect("read override file");
    assert_eq!(rendered, "python layer\n");
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

fn cli_binary() -> &'static Path {
    static CLI_BINARY: OnceLock<PathBuf> = OnceLock::new();
    CLI_BINARY
        .get_or_init(|| PathBuf::from(env!("CARGO_BIN_EXE_omne-project-init")))
        .as_path()
}

fn repo_template_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn template_fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
        let path = env::temp_dir().join(format!("init-regression-{prefix}-{nanos}-{unique}"));
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

struct PathCleanup {
    path: PathBuf,
}

impl PathCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for PathCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
