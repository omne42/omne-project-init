use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn root_layout_docs_only_change_does_not_require_changelog() {
    let repo = init_repo("root-docs-only", &["--project", "rust", "--layout", "root"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::write(repo.path().join("README.md"), "# updated\n").expect("write README");
    git_add(repo.path(), &["README.md"]);

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn pre_commit_checks_staged_snapshot() {
    if !command_works("node", &["--version"]) {
        eprintln!("skipping staged snapshot regression: node not found");
        return;
    }

    let repo = init_repo("node-staged-snapshot", &["--project", "nodejs"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::write(
        repo.path().join("src/index.js"),
        "export function greet(name) {\n",
    )
    .expect("write invalid JS");
    fs::write(
        repo.path().join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n- keep staging regression covered\n",
    )
    .expect("write changelog");
    git_add(repo.path(), &["src/index.js", "CHANGELOG.md"]);

    fs::write(
        repo.path().join("src/index.js"),
        "export function greet(name) {\n  return `hello, ${name}`;\n}\n",
    )
    .expect("write unstaged fix");

    let error = run_generated_repo_check_fail(repo.path(), &["pre-commit"]);
    assert!(
        error.contains("node syntax"),
        "expected staged snapshot syntax failure, got: {error}"
    );
}

#[test]
fn commit_msg_detects_top_level_node_major_bump() {
    let repo = init_repo("node-major-bump", &["--project", "nodejs"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::write(
        repo.path().join("package.json"),
        concat!(
            "{\n",
            "  \"meta\": { \"version\": \"0.0.1\" },\n",
            "  \"version\": \"1.2.3\",\n",
            "  \"name\": \"node-major-bump\",\n",
            "  \"type\": \"module\",\n",
            "  \"scripts\": { \"test\": \"node --test\" }\n",
            "}\n"
        ),
    )
    .expect("write package.json");
    git_add(repo.path(), &["package.json"]);
    commit_all(repo.path(), "chore(node): prepare major baseline");

    fs::write(
        repo.path().join("package.json"),
        concat!(
            "{\n",
            "  \"meta\": { \"version\": \"0.0.1\" },\n",
            "  \"version\": \"2.0.0\",\n",
            "  \"name\": \"node-major-bump\",\n",
            "  \"type\": \"module\",\n",
            "  \"scripts\": { \"test\": \"node --test\" }\n",
            "}\n"
        ),
    )
    .expect("write bumped package.json");
    git_add(repo.path(), &["package.json"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.test");
    fs::write(&commit_msg, "feat(node): major bump without marker\n").expect("write commit msg");
    let error = run_generated_repo_check_fail(
        repo.path(),
        &[
            "commit-msg",
            "--commit-msg-file",
            commit_msg.to_string_lossy().as_ref(),
        ],
    );
    assert!(
        error.contains("requires an explicit breaking commit message"),
        "expected breaking marker failure, got: {error}"
    );
}

#[test]
fn prerelease_versions_are_accepted_by_commit_msg_gate() {
    let repo = init_repo("rust-prerelease", &["--project", "rust", "--layout", "root"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    replace_in_file(repo.path().join("Cargo.toml"), "version = \"0.1.0\"", "version = \"1.0.0-alpha.1\"");
    git_add(repo.path(), &["Cargo.toml"]);
    commit_all(repo.path(), "feat(repo)!: enter prerelease");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"1.0.0-alpha.1\"",
        "version = \"1.0.1-beta.1\"",
    );
    git_add(repo.path(), &["Cargo.toml"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.test");
    fs::write(&commit_msg, "fix(repo): prerelease patch\n").expect("write commit msg");
    run_generated_repo_check(
        repo.path(),
        &[
            "commit-msg",
            "--commit-msg-file",
            commit_msg.to_string_lossy().as_ref(),
        ],
    );
}

#[test]
fn root_layout_uses_configured_changelog_path() {
    let repo = init_repo("root-config-changelog", &["--project", "rust", "--layout", "root"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::create_dir_all(repo.path().join("docs")).expect("create docs dir");
    let changelog_text =
        fs::read_to_string(repo.path().join("CHANGELOG.md")).expect("read original changelog");
    fs::write(repo.path().join("docs/CHANGELOG.md"), changelog_text).expect("write moved changelog");
    fs::remove_file(repo.path().join("CHANGELOG.md")).expect("remove original changelog");
    replace_in_file(
        repo.path().join("repo-check.toml"),
        "changelog_path = \"CHANGELOG.md\"",
        "changelog_path = \"docs/CHANGELOG.md\"",
    );
    let src_main = repo.path().join("src/main.rs");
    let mut src_text = fs::read_to_string(&src_main).expect("read src/main.rs");
    src_text.push_str("// configured changelog path regression\n");
    fs::write(&src_main, src_text).expect("write src/main.rs");
    git_add(
        repo.path(),
        &["repo-check.toml", "docs/CHANGELOG.md", "CHANGELOG.md", "src/main.rs"],
    );

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn crate_layout_root_governance_changes_require_primary_changelog() {
    let repo = init_repo("crate-root-changelog", &["--project", "rust", "--layout", "crate"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::write(repo.path().join("README.md"), "# governance change\n").expect("write README");
    git_add(repo.path(), &["README.md"]);

    let error = run_generated_repo_check_fail(repo.path(), &["pre-commit"]);
    assert!(
        error.contains(&format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()))),
        "expected primary crate changelog requirement, got: {error}"
    );
}

#[test]
fn hook_templates_recognize_windows_absolute_manifest_paths() {
    let pre_commit = fs::read_to_string("templates/common/githooks/pre-commit")
        .expect("read pre-commit hook template");
    let commit_msg = fs::read_to_string("templates/common/githooks/commit-msg")
        .expect("read commit-msg hook template");

    for text in [&pre_commit, &commit_msg] {
        assert!(text.contains("[A-Za-z]:/*"), "missing drive-letter path detection");
        assert!(text.contains("[A-Za-z]:\\\\*"), "missing backslash path detection");
        assert!(text.contains("\\\\\\\\*"), "missing UNC path detection");
    }
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

fn git_init(repo_root: &Path) {
    run_git(repo_root, &["init", "-b", "main"]);
    run_git(repo_root, &["config", "user.name", "Smoke Test"]);
    run_git(repo_root, &["config", "user.email", "smoke@example.com"]);
    run_git(repo_root, &["checkout", "-b", "feat/regression"]);
}

fn commit_all(repo_root: &Path, message: &str) {
    run_git(repo_root, &["add", "."]);
    run_git(repo_root, &["commit", "-m", message]);
}

fn git_add(repo_root: &Path, paths: &[&str]) {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).arg("add").arg("--");
    for path in paths {
        command.arg(path);
    }
    run_ok("git add", &mut command);
}

fn run_git(repo_root: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(args);
    run_ok("git", &mut command);
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
    let mut command = generated_repo_check_command(repo_root, args);
    run_ok("generated repo-check", &mut command)
}

fn run_generated_repo_check_fail(repo_root: &Path, args: &[&str]) -> String {
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
        .arg(repo_root.join(".generated-target"))
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

fn replace_in_file(path: PathBuf, from: &str, to: &str) {
    let text = fs::read_to_string(&path).expect("read file for replacement");
    let updated = text.replace(from, to);
    assert_ne!(text, updated, "replacement target not found in {}", path.display());
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

fn command_works(program: &str, args: &[&str]) -> bool {
    matches!(
        Command::new(program).args(args).output(),
        Ok(output) if output.status.success()
    )
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
        let path = env::temp_dir().join(format!("repo-check-regression-{prefix}-{nanos}-{unique}"));
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
