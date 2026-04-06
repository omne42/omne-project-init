use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn manifest_and_init_ignore_untracked_template_files() {
    let _guard = template_tree_lock().lock().expect("lock template tree");
    let artifact = repo_root()
        .join("templates")
        .join("common")
        .join("__review_untracked_template.bin");
    let _artifact = ScopedFile::create(&artifact, &[0xff, 0xfe, 0x00, b'x']);

    let manifest_target = TempDir::new("manifest-untracked-template");
    let output = run_cli([
        "manifest",
        manifest_target.path().to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "root",
    ]);
    assert!(!output.contains("__review_untracked_template.bin"));
    assert!(!output.contains('\\'));

    let init_target = TempDir::new("init-untracked-template");
    run_cli([
        "init",
        init_target.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--no-git-init",
    ]);
    assert!(!init_target.path().join("__review_untracked_template.bin").exists());
}

#[test]
fn force_replaces_scaffold_without_touching_unmanaged_files() {
    let repo = init_repo(
        "force-replaces-scaffold",
        &["--project", "rust", "--layout", "crate"],
    );
    let keep_path = repo.path().join("keep.txt");
    fs::write(&keep_path, "keep").expect("write unmanaged file");

    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
    ]);

    let python_package = repo_slug(repo.path()).replace('-', "_");
    assert!(repo.path().join("pyproject.toml").is_file());
    assert!(repo.path().join(python_package).join("__init__.py").is_file());
    assert!(!repo.path().join("Cargo.toml").exists());
    assert!(!repo.path().join("crates").exists());
    assert!(keep_path.is_file());

    let repo_check = fs::read_to_string(repo.path().join("repo-check.toml"))
        .expect("read regenerated repo-check.toml");
    assert!(repo_check.contains("project_kind = \"python\""));
    assert!(repo_check.contains("layout = \"root\""));
}

#[test]
fn rust_agents_point_to_validation_command_instead_of_fake_test_path() {
    let root_repo = init_repo("rust-root-agents", &["--project", "rust", "--layout", "root"]);
    let crate_repo = init_repo("rust-crate-agents", &["--project", "rust", "--layout", "crate"]);

    let root_agents =
        fs::read_to_string(root_repo.path().join("AGENTS.md")).expect("read root AGENTS.md");
    let crate_agents =
        fs::read_to_string(crate_repo.path().join("AGENTS.md")).expect("read crate AGENTS.md");

    for agents in [&root_agents, &crate_agents] {
        assert!(agents.contains("主要验证命令：`cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local`"));
        assert!(!agents.contains("主要测试入口"));
    }
}

fn cli_binary() -> &'static str {
    env!("CARGO_BIN_EXE_omne-project-init")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn repo_slug(path: &Path) -> &str {
    path.file_name()
        .and_then(|value| value.to_str())
        .expect("temp repo path must end with a UTF-8 file name")
}

fn template_tree_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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

struct ScopedFile {
    path: PathBuf,
}

impl ScopedFile {
    fn create(path: &Path, content: &[u8]) -> Self {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create scoped file parent");
        }
        fs::write(path, content).expect("write scoped file");
        Self {
            path: path.to_path_buf(),
        }
    }
}

impl Drop for ScopedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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
