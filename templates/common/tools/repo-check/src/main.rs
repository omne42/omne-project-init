use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

const ALLOWED_BRANCH_PREFIXES: &[&str] = &[
    "feat/",
    "fix/",
    "docs/",
    "refactor/",
    "perf/",
    "test/",
    "chore/",
    "build/",
    "ci/",
    "revert/",
];

const ALLOWED_COMMIT_TYPES: &[&str] = &[
    "feat", "fix", "docs", "refactor", "perf", "test", "chore", "build", "ci", "revert",
];
const REPO_CHECK_SCHEMA_VERSION: &str = "__REPO_CHECK_SCHEMA_VERSION__";
const WORKTREE_SNAPSHOT_IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    ".generated-target",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectKind {
    Rust,
    Python,
    Nodejs,
}

impl ProjectKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "rust" => Ok(Self::Rust),
            "python" => Ok(Self::Python),
            "nodejs" => Ok(Self::Nodejs),
            _ => Err(format!("repo-check: unsupported project kind: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Root,
    Crate,
}

impl Layout {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "root" => Ok(Self::Root),
            "crate" => Ok(Self::Crate),
            _ => Err(format!("repo-check: unsupported layout: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceMode {
    Local,
    Ci,
}

impl WorkspaceMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "local" => Ok(Self::Local),
            "ci" => Ok(Self::Ci),
            _ => Err(format!("repo-check: unsupported workspace mode: {value}")),
        }
    }
}

#[derive(Debug)]
enum CliCommand {
    InstallHooks {
        repo_root: PathBuf,
    },
    PreCommit {
        repo_root: PathBuf,
    },
    CommitMsg {
        repo_root: PathBuf,
        commit_msg_file: PathBuf,
    },
    Workspace {
        repo_root: PathBuf,
        mode: WorkspaceMode,
    },
    ValidateBranch {
        repo_root: PathBuf,
    },
}

#[derive(Clone, Debug)]
struct RepoConfig {
    template_version: String,
    schema_version: String,
    repo_name: String,
    project_kind: ProjectKind,
    layout: Layout,
    package_name: String,
    crate_dir: String,
    python_package: String,
    package_manifest_path: String,
    changelog_path: String,
    primary_source_path: String,
}

#[derive(Clone, Debug)]
struct CrateLayoutPaths {
    container_dir: PathBuf,
    manifest_name: String,
    changelog_name: String,
}

#[derive(Clone, Debug)]
struct WorkspaceManifestLocation {
    path: String,
    text: String,
}

#[derive(Debug)]
struct StagedState {
    paths: Vec<String>,
    deleted_paths: BTreeSet<String>,
    changelog_paths: Vec<String>,
    non_changelog_count: usize,
    active_crate_dirs: BTreeSet<String>,
    crate_dirs_with_non_changelog_changes: BTreeSet<String>,
}

#[derive(Debug)]
struct VersionTarget {
    label: String,
    path: String,
    old_version: Option<String>,
    new_version: Option<String>,
}

#[derive(Debug)]
struct ParsedCommitMessage {
    breaking: bool,
}

#[derive(Clone, Debug)]
struct PythonRuntime {
    command: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PythonVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

fn main() {
    if let Err(error) = real_main() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), String> {
    let command = parse_cli(env::args_os().skip(1))?;
    match command {
        CliCommand::InstallHooks { repo_root } => install_hooks(&normalize_repo_root(repo_root)),
        CliCommand::PreCommit { repo_root } => {
            let repo_root = normalize_repo_root(repo_root);
            let config = RepoConfig::load(&repo_root)?;
            validate_layout_shape(&repo_root, &config)?;
            run_pre_commit(&repo_root, &config)
        }
        CliCommand::CommitMsg {
            repo_root,
            commit_msg_file,
        } => {
            let repo_root = normalize_repo_root(repo_root);
            let config = RepoConfig::load(&repo_root)?;
            validate_layout_shape(&repo_root, &config)?;
            run_commit_msg(&repo_root, &config, &commit_msg_file)
        }
        CliCommand::Workspace { repo_root, mode } => {
            let repo_root = normalize_repo_root(repo_root);
            let config = RepoConfig::load(&repo_root)?;
            validate_layout_shape(&repo_root, &config)?;
            run_workspace_checks_on_worktree_snapshot(&repo_root, &config, mode)
        }
        CliCommand::ValidateBranch { repo_root } => {
            validate_branch_name(&normalize_repo_root(repo_root))
        }
    }
}

fn parse_cli(args: impl Iterator<Item = OsString>) -> Result<CliCommand, String> {
    let values: Vec<OsString> = args.collect();
    if values.is_empty() {
        return Err(usage());
    }

    let subcommand = utf8_arg(&values[0], "subcommand")?;
    let mut repo_root = PathBuf::from(".");
    let mut commit_msg_file: Option<PathBuf> = None;
    let mut workspace_mode: Option<WorkspaceMode> = None;

    let mut index = 1;
    if subcommand == "workspace" {
        let value = values.get(index).ok_or_else(usage)?;
        workspace_mode = Some(WorkspaceMode::parse(utf8_arg(value, "workspace mode")?)?);
        index += 1;
    }

    while index < values.len() {
        let current = &values[index];
        match utf8_arg(current, "argument")? {
            "--repo-root" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                repo_root = PathBuf::from(value);
            }
            "--commit-msg-file" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                commit_msg_file = Some(PathBuf::from(value));
            }
            value => {
                return Err(format!(
                    "repo-check: unsupported argument: {value}\n\n{}",
                    usage()
                ));
            }
        }
        index += 1;
    }

    match subcommand {
        "install-hooks" => Ok(CliCommand::InstallHooks { repo_root }),
        "pre-commit" => Ok(CliCommand::PreCommit { repo_root }),
        "commit-msg" => Ok(CliCommand::CommitMsg {
            repo_root,
            commit_msg_file: commit_msg_file.ok_or_else(usage)?,
        }),
        "workspace" => Ok(CliCommand::Workspace {
            repo_root,
            mode: workspace_mode.ok_or_else(usage)?,
        }),
        "validate-branch" => Ok(CliCommand::ValidateBranch { repo_root }),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: repo-check <install-hooks|pre-commit|commit-msg|workspace|validate-branch> [workspace local|ci] [--repo-root PATH] [--commit-msg-file PATH]".to_string()
}

fn utf8_arg<'a>(value: &'a OsString, label: &str) -> Result<&'a str, String> {
    value.to_str().ok_or_else(|| {
        format!(
            "repo-check: {label} must be valid UTF-8: {}",
            PathBuf::from(value).display()
        )
    })
}

fn normalize_repo_root(path: PathBuf) -> PathBuf {
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map(|current_dir| current_dir.join(&path))
            .unwrap_or(path)
    };
    let path = path.canonicalize().unwrap_or(path);
    find_repo_root(&path)
        .or_else(|| git_toplevel_for(&path).and_then(|repo_root| find_repo_root(&repo_root)))
        .unwrap_or(path)
}

fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    start
        .ancestors()
        .find(|candidate| candidate.join("repo-check.toml").is_file())
        .map(Path::to_path_buf)
}

fn git_toplevel_for(path: &Path) -> Option<PathBuf> {
    let working_dir = if path.is_dir() { path } else { path.parent()? };
    let output = Command::new("git")
        .arg("-C")
        .arg(working_dir)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let repo_root = stdout.trim();
    if repo_root.is_empty() {
        return None;
    }
    Some(PathBuf::from(repo_root))
}

impl RepoConfig {
    fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join("repo-check.toml");
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("repo-check: failed to read {}: {error}", path.display()))?;
        Self::parse_from_text(&text, &path.display().to_string())
    }

    fn load_from_git(repo_root: &Path, revision: &str) -> Result<Option<Self>, String> {
        let pathspec = format!("{revision}:repo-check.toml");
        let Some(text) = git_show_text(repo_root, &pathspec)? else {
            return Ok(None);
        };
        Self::parse_from_text(&text, &pathspec).map(Some)
    }

    fn parse_from_text(text: &str, source: &str) -> Result<Self, String> {
        let values = toml::from_str::<TomlValue>(text)
            .map_err(|error| format!("repo-check: failed to parse {source}: {error}"))?;
        Ok(Self {
            template_version: required_value(&values, "template_version")?,
            schema_version: required_schema_version(&values)?,
            repo_name: required_value(&values, "repo_name")?,
            project_kind: ProjectKind::parse(&required_value(&values, "project_kind")?)?,
            layout: Layout::parse(&required_value(&values, "layout")?)?,
            package_name: required_value(&values, "package_name")?,
            crate_dir: required_value(&values, "crate_dir")?,
            python_package: required_value(&values, "python_package")?,
            package_manifest_path: required_value(&values, "package_manifest_path")?,
            changelog_path: required_value(&values, "changelog_path")?,
            primary_source_path: required_value(&values, "primary_source_path")?,
        })
    }
}

fn required_value(values: &TomlValue, key: &str) -> Result<String, String> {
    values
        .get(key)
        .and_then(TomlValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("repo-check: missing `{key}` in repo-check.toml"))
}

fn required_schema_version(values: &TomlValue) -> Result<String, String> {
    let schema_version = required_value(values, "schema_version")?;
    if schema_version == REPO_CHECK_SCHEMA_VERSION {
        return Ok(schema_version);
    }
    Err(format!(
        "repo-check: unsupported repo-check.toml schema_version `{schema_version}` (expected `{REPO_CHECK_SCHEMA_VERSION}`)"
    ))
}

fn install_hooks(repo_root: &Path) -> Result<(), String> {
    git_output(repo_root, &["rev-parse", "--show-toplevel"], false)?;

    let pre_commit = repo_root.join("githooks").join("pre-commit");
    let commit_msg = repo_root.join("githooks").join("commit-msg");
    if !pre_commit.is_file() {
        return Err(format!(
            "repo-check: missing hook file: {}",
            pre_commit.display()
        ));
    }
    if !commit_msg.is_file() {
        return Err(format!(
            "repo-check: missing hook file: {}",
            commit_msg.display()
        ));
    }

    maybe_mark_executable(&pre_commit)?;
    maybe_mark_executable(&commit_msg)?;
    git_output(
        repo_root,
        &["config", "--local", "core.hooksPath", "githooks"],
        false,
    )?;

    println!("Configured git hooks: core.hooksPath=githooks");
    println!("Hooks enabled: pre-commit, commit-msg");
    Ok(())
}

fn maybe_mark_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| format!("repo-check: failed to stat {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(|error| {
            format!(
                "repo-check: failed to set executable permission on {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn run_pre_commit(repo_root: &Path, config: &RepoConfig) -> Result<(), String> {
    validate_branch_name(repo_root)?;
    let staged = collect_staged_state(repo_root, config)?;
    if staged.paths.is_empty() {
        return Ok(());
    }

    require_major_bump_override(repo_root, config)?;
    validate_allowed_changelog_paths(config, &staged)?;
    validate_required_changelog_not_deleted(config, &staged)?;
    validate_changelog_update(repo_root, config, &staged)?;
    validate_not_changelog_only(&staged)?;
    validate_released_sections_immutable(repo_root, config, &staged)?;

    run_workspace_checks_on_staged_snapshot(repo_root, config, WorkspaceMode::Local)
}

fn run_commit_msg(
    repo_root: &Path,
    config: &RepoConfig,
    commit_msg_file: &Path,
) -> Result<(), String> {
    validate_branch_name(repo_root)?;
    let commit_message = read_commit_message(commit_msg_file)?;
    let first_line = commit_message
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
        .to_string();
    if is_special_commit_message(&first_line) {
        return Ok(());
    }

    let parsed = parse_conventional_commit(&commit_message)?;
    require_breaking_commit_marker(repo_root, config, &parsed)
}

fn validate_layout_shape(repo_root: &Path, config: &RepoConfig) -> Result<(), String> {
    if config.layout == Layout::Crate && config.project_kind != ProjectKind::Rust {
        return Err("repo-check: crate layout is only supported for rust projects".to_string());
    }

    let package_manifest = validate_required_configured_file(
        repo_root,
        &config.package_manifest_path,
        "package_manifest_path",
    )?;
    let changelog =
        validate_required_configured_file(repo_root, &config.changelog_path, "changelog_path")?;
    validate_package_manifest_shape(&package_manifest, config)?;
    validate_changelog_shape(&changelog, &config.changelog_path)?;

    match (config.project_kind, config.layout) {
        (ProjectKind::Rust, Layout::Root) => {
            validate_required_configured_file(
                repo_root,
                &config.primary_source_path,
                "primary_source_path",
            )?;
        }
        (ProjectKind::Rust, Layout::Crate) => {
            let layout_paths = crate_layout_paths(config)?;
            let workspace_manifest =
                live_workspace_manifest_location(repo_root, config)?.ok_or_else(|| {
                    format!(
                        "repo-check: rust crate layout requires a workspace manifest reachable from {}.",
                        config.package_manifest_path
                    )
                })?;
            let crate_dirs =
                discover_crate_dirs(repo_root, &layout_paths, &workspace_manifest.path)?;
            if crate_dirs.is_empty() {
                return Err(
                    "repo-check: rust crate layout requires at least one configured package manifest"
                        .to_string(),
                );
            }
            if !crate_dirs
                .iter()
                .any(|crate_dir| crate_dir == &config.crate_dir)
            {
                return Err(format!(
                    "repo-check: crate layout config drift: crate_dir `{}` is not an active crate under {}.",
                    config.crate_dir,
                    layout_paths.container_dir.display()
                ));
            }

            validate_primary_source_in_primary_crate(repo_root, config, &layout_paths)?;
        }
        (ProjectKind::Python, Layout::Root) => {
            validate_required_configured_file(
                repo_root,
                &config.primary_source_path,
                "primary_source_path",
            )?;
        }
        (ProjectKind::Nodejs, Layout::Root) => {
            validate_required_configured_file(
                repo_root,
                &config.primary_source_path,
                "primary_source_path",
            )?;
        }
        _ => {}
    }

    Ok(())
}

fn validate_package_manifest_shape(
    package_manifest: &Path,
    config: &RepoConfig,
) -> Result<(), String> {
    let text = fs::read_to_string(package_manifest).map_err(|error| {
        format!(
            "repo-check: failed to read {}: {error}",
            package_manifest.display()
        )
    })?;
    match config.project_kind {
        ProjectKind::Rust => {
            validate_rust_package_manifest_shape(&text, &config.package_manifest_path)
        }
        ProjectKind::Python => {
            validate_python_package_manifest_shape(&text, &config.package_manifest_path)
        }
        ProjectKind::Nodejs => {
            validate_node_package_manifest_shape(&text, &config.package_manifest_path)
        }
    }
}

fn validate_rust_package_manifest_shape(text: &str, manifest_path: &str) -> Result<(), String> {
    let parsed = cargo_toml(Some(text)).ok_or_else(|| {
        format!(
            "repo-check: configured package_manifest_path must point to a valid Cargo manifest: {manifest_path}"
        )
    })?;
    let Some(package) = parsed.get("package").and_then(TomlValue::as_table) else {
        return Err(format!(
            "repo-check: configured package_manifest_path must point to a Cargo package manifest with `package.name` and version fields: {manifest_path}"
        ));
    };
    let has_name = package.get("name").and_then(TomlValue::as_str).is_some();
    let has_version = match package.get("version") {
        Some(version) if version.as_str().is_some() => true,
        Some(version) => {
            version
                .as_table()
                .and_then(|table| table.get("workspace"))
                .and_then(TomlValue::as_bool)
                == Some(true)
        }
        None => false,
    };
    if has_name && has_version {
        return Ok(());
    }
    Err(format!(
        "repo-check: configured package_manifest_path must point to a Cargo package manifest with `package.name` and version fields: {manifest_path}"
    ))
}

fn validate_python_package_manifest_shape(text: &str, manifest_path: &str) -> Result<(), String> {
    let parsed = cargo_toml(Some(text)).ok_or_else(|| {
        format!(
            "repo-check: configured package_manifest_path must point to a valid pyproject.toml: {manifest_path}"
        )
    })?;
    let Some(project) = parsed.get("project").and_then(TomlValue::as_table) else {
        return Err(format!(
            "repo-check: configured package_manifest_path must point to a pyproject.toml with string `project.name` and `project.version`: {manifest_path}"
        ));
    };
    if project.get("name").and_then(TomlValue::as_str).is_some()
        && project.get("version").and_then(TomlValue::as_str).is_some()
    {
        return Ok(());
    }
    Err(format!(
        "repo-check: configured package_manifest_path must point to a pyproject.toml with string `project.name` and `project.version`: {manifest_path}"
    ))
}

fn validate_node_package_manifest_shape(text: &str, manifest_path: &str) -> Result<(), String> {
    let parsed = serde_json::from_str::<JsonValue>(text).map_err(|error| {
        format!(
            "repo-check: configured package_manifest_path must point to a valid package.json: {manifest_path} ({error})"
        )
    })?;
    let has_name = parsed.get("name").and_then(JsonValue::as_str).is_some();
    let has_version = parsed.get("version").and_then(JsonValue::as_str).is_some();
    if has_name && has_version {
        return Ok(());
    }
    Err(format!(
        "repo-check: configured package_manifest_path must point to a package.json with top-level string `name` and `version` fields: {manifest_path}"
    ))
}

fn validate_changelog_shape(changelog: &Path, changelog_path: &str) -> Result<(), String> {
    let text = fs::read_to_string(changelog).map_err(|error| {
        format!(
            "repo-check: failed to read {}: {error}",
            changelog.display()
        )
    })?;
    if text
        .lines()
        .any(|line| line.trim_end_matches('\r') == "## [Unreleased]")
    {
        return Ok(());
    }
    Err(format!(
        "repo-check: configured changelog_path must contain a `## [Unreleased]` section: {changelog_path}"
    ))
}

fn validate_required_configured_file(
    repo_root: &Path,
    raw_path: &str,
    key: &str,
) -> Result<PathBuf, String> {
    let validated = validate_repo_relative_path(raw_path, key)?;
    let absolute = repo_root.join(&validated);
    if absolute.is_file() {
        return Ok(absolute);
    }
    Err(format!(
        "repo-check: configured {key} does not exist as a file: {raw_path}"
    ))
}

fn validate_repo_relative_path(raw_path: &str, key: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw_path);
    let mut saw_normal_component = false;

    for component in path.components() {
        match component {
            Component::Normal(_) => saw_normal_component = true,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(format!(
                    "repo-check: {key} must be a normalized repository-relative path: {raw_path}"
                ));
            }
        }
    }

    if !saw_normal_component {
        return Err(format!(
            "repo-check: {key} must be a normalized repository-relative path: {raw_path}"
        ));
    }

    Ok(path.to_path_buf())
}

fn validate_primary_source_in_primary_crate(
    repo_root: &Path,
    config: &RepoConfig,
    layout_paths: &CrateLayoutPaths,
) -> Result<(), String> {
    let primary_source = validate_required_configured_file(
        repo_root,
        &config.primary_source_path,
        "primary_source_path",
    )?;
    let primary_crate_root = repo_root
        .join(&layout_paths.container_dir)
        .join(&config.crate_dir);
    if primary_source.starts_with(&primary_crate_root) {
        return Ok(());
    }

    Err(format!(
        "repo-check: primary_source_path `{}` must live under the primary crate directory {}",
        config.primary_source_path,
        normalize_repo_relative_path(
            primary_crate_root
                .strip_prefix(repo_root)
                .unwrap_or(&primary_crate_root)
        )
    ))
}

fn validate_branch_name(repo_root: &Path) -> Result<(), String> {
    let Some(branch) = git_current_branch(repo_root)? else {
        return Ok(());
    };
    if branch == "main" || branch == "master" {
        return Ok(());
    }
    if ALLOWED_BRANCH_PREFIXES
        .iter()
        .any(|prefix| branch.starts_with(prefix))
    {
        return Ok(());
    }

    Err(format!(
        "repo-check: invalid branch name: {branch}\n\nBranch must be `main`, `master`, or start with one of:\n- {}",
        ALLOWED_BRANCH_PREFIXES.join("\n- ")
    ))
}

fn git_current_branch(repo_root: &Path) -> Result<Option<String>, String> {
    let args = ["symbolic-ref", "--quiet", "--short", "HEAD"];
    let output = run_git(repo_root, &args)?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            return Ok(None);
        }
        return Ok(Some(branch));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }

    Err(render_git_failure(repo_root, &args, &output))
}

fn collect_staged_state(repo_root: &Path, config: &RepoConfig) -> Result<StagedState, String> {
    let changed = git_output(
        repo_root,
        &[
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "--diff-filter=ACMRD",
        ],
        false,
    )?;
    let deleted = git_output(
        repo_root,
        &["diff", "--cached", "--name-only", "-z", "--diff-filter=D"],
        false,
    )?;

    let active_crate_dirs = staged_active_crate_dirs(repo_root, config)?;
    let paths = split_null_terminated(&changed);
    let deleted_paths: BTreeSet<String> = split_null_terminated(&deleted).into_iter().collect();
    let changelog_paths: Vec<String> = paths
        .iter()
        .filter(|path| is_changelog_path(config, path))
        .cloned()
        .collect();
    let non_changelog_count = paths
        .iter()
        .filter(|path| !is_changelog_path(config, path))
        .count();
    let crate_dirs_with_non_changelog_changes = paths
        .iter()
        .filter(|path| !is_changelog_path(config, path))
        .filter_map(|path| active_crate_dir_for_path(config, &active_crate_dirs, path))
        .collect();

    Ok(StagedState {
        paths,
        deleted_paths,
        changelog_paths,
        non_changelog_count,
        active_crate_dirs,
        crate_dirs_with_non_changelog_changes,
    })
}

fn split_null_terminated(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn is_changelog_path(config: &RepoConfig, path: &str) -> bool {
    path == config.changelog_path
        || crate_layout_paths(config)
            .ok()
            .is_some_and(|layout_paths| is_package_changelog_path(path, &layout_paths))
}

fn crate_layout_paths(config: &RepoConfig) -> Result<CrateLayoutPaths, String> {
    let manifest_path = Path::new(&config.package_manifest_path);
    let changelog_path = Path::new(&config.changelog_path);
    let manifest_parent = manifest_path.parent().ok_or_else(|| {
        format!(
            "repo-check: invalid crate layout package_manifest_path: {}",
            config.package_manifest_path
        )
    })?;
    let changelog_parent = changelog_path.parent().ok_or_else(|| {
        format!(
            "repo-check: invalid crate layout changelog_path: {}",
            config.changelog_path
        )
    })?;
    let manifest_crate_dir = manifest_parent
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "repo-check: invalid crate layout package_manifest_path: {}",
                config.package_manifest_path
            )
        })?;
    let changelog_crate_dir = changelog_parent
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "repo-check: invalid crate layout changelog_path: {}",
                config.changelog_path
            )
        })?;
    if manifest_crate_dir != config.crate_dir || changelog_crate_dir != config.crate_dir {
        return Err(format!(
            "repo-check: crate layout config drift: crate_dir `{}` must match package/changelog parents.",
            config.crate_dir
        ));
    }
    let manifest_container = manifest_parent.parent().ok_or_else(|| {
        format!(
            "repo-check: invalid crate layout package_manifest_path: {}",
            config.package_manifest_path
        )
    })?;
    let changelog_container = changelog_parent.parent().ok_or_else(|| {
        format!(
            "repo-check: invalid crate layout changelog_path: {}",
            config.changelog_path
        )
    })?;
    if manifest_container != changelog_container {
        return Err(format!(
            "repo-check: crate layout config drift: package_manifest_path {} and changelog_path {} must live under the same package container.",
            config.package_manifest_path, config.changelog_path
        ));
    }
    let manifest_name = manifest_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "repo-check: invalid crate layout package_manifest_path: {}",
                config.package_manifest_path
            )
        })?;
    let changelog_name = changelog_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "repo-check: invalid crate layout changelog_path: {}",
                config.changelog_path
            )
        })?;
    Ok(CrateLayoutPaths {
        container_dir: manifest_container.to_path_buf(),
        manifest_name: manifest_name.to_string(),
        changelog_name: changelog_name.to_string(),
    })
}

fn workspace_manifest_candidates(config: &RepoConfig) -> Result<Vec<String>, String> {
    let manifest_path = Path::new(&config.package_manifest_path);
    let crate_dir = manifest_path.parent().ok_or_else(|| {
        format!(
            "repo-check: invalid crate layout package_manifest_path: {}",
            config.package_manifest_path
        )
    })?;

    let mut candidates = Vec::new();
    let mut current = crate_dir.parent();
    while let Some(directory) = current {
        let candidate = normalize_repo_relative_path(&directory.join("Cargo.toml"));
        if !candidate.is_empty() && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
        current = directory.parent();
    }

    if candidates.is_empty() {
        return Err(format!(
            "repo-check: crate layout package_manifest_path {} must live under a workspace container.",
            config.package_manifest_path
        ));
    }

    Ok(candidates)
}

fn normalize_repo_relative_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

fn workspace_manifest_parent(path: &str) -> PathBuf {
    Path::new(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn live_workspace_manifest_location(
    repo_root: &Path,
    config: &RepoConfig,
) -> Result<Option<WorkspaceManifestLocation>, String> {
    for candidate in workspace_manifest_candidates(config)? {
        let full_path = repo_root.join(&candidate);
        if !full_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&full_path).map_err(|error| {
            format!(
                "repo-check: failed to read {}: {error}",
                full_path.display()
            )
        })?;
        let has_workspace = cargo_toml(Some(&text))
            .and_then(|parsed| parsed.get("workspace").cloned())
            .is_some();
        if has_workspace {
            return Ok(Some(WorkspaceManifestLocation {
                path: candidate,
                text,
            }));
        }
    }
    Ok(None)
}

fn git_workspace_manifest_location(
    repo_root: &Path,
    config: &RepoConfig,
    revision: &str,
) -> Result<Option<WorkspaceManifestLocation>, String> {
    for candidate in workspace_manifest_candidates(config)? {
        let spec = format!("{revision}:{candidate}");
        let Some(text) = git_show_text(repo_root, &spec)? else {
            continue;
        };
        let has_workspace = cargo_toml(Some(&text))
            .and_then(|parsed| parsed.get("workspace").cloned())
            .is_some();
        if has_workspace {
            return Ok(Some(WorkspaceManifestLocation {
                path: candidate,
                text,
            }));
        }
    }
    Ok(None)
}

fn package_dir_for_path(config: &RepoConfig, path: &str) -> Option<String> {
    let layout_paths = crate_layout_paths(config).ok()?;
    let path = Path::new(path);
    let container = layout_paths.container_dir.as_path();
    let relative = path.strip_prefix(container).ok()?;
    let mut parts = relative.components();
    let package_dir = parts.next()?.as_os_str().to_str()?.to_string();
    if package_dir.is_empty() {
        return None;
    }
    Some(package_dir)
}

fn package_manifest_path(config: &RepoConfig, crate_dir: &str) -> String {
    if let Ok(layout_paths) = crate_layout_paths(config) {
        return layout_paths
            .container_dir
            .join(crate_dir)
            .join(&layout_paths.manifest_name)
            .to_string_lossy()
            .replace('\\', "/");
    }
    config.package_manifest_path.clone()
}

fn package_changelog_path(config: &RepoConfig, crate_dir: &str) -> String {
    if let Ok(layout_paths) = crate_layout_paths(config) {
        return layout_paths
            .container_dir
            .join(crate_dir)
            .join(&layout_paths.changelog_name)
            .to_string_lossy()
            .replace('\\', "/");
    }
    config.changelog_path.clone()
}

fn package_manifest_path_with_layout(layout_paths: &CrateLayoutPaths, crate_dir: &str) -> String {
    layout_paths
        .container_dir
        .join(crate_dir)
        .join(&layout_paths.manifest_name)
        .to_string_lossy()
        .replace('\\', "/")
}

fn package_dir_from_changelog_path(config: &RepoConfig, path: &str) -> Option<String> {
    let layout_paths = crate_layout_paths(config).ok()?;
    is_package_changelog_path(path, &layout_paths).then(|| package_dir_for_path(config, path))?
}

fn is_package_changelog_path(path: &str, layout_paths: &CrateLayoutPaths) -> bool {
    let Ok(relative) = Path::new(path).strip_prefix(&layout_paths.container_dir) else {
        return false;
    };
    let mut parts = relative.components();
    let Some(crate_dir) = parts.next() else {
        return false;
    };
    let Some(changelog_name) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    matches!(crate_dir, std::path::Component::Normal(_))
        && changelog_name.as_os_str() == std::ffi::OsStr::new(&layout_paths.changelog_name)
}

fn active_crate_dir_for_path(
    config: &RepoConfig,
    active_crate_dirs: &BTreeSet<String>,
    path: &str,
) -> Option<String> {
    let crate_dir = package_dir_for_path(config, path)?;
    active_crate_dirs.contains(&crate_dir).then_some(crate_dir)
}

fn crate_manifest_deleted(config: &RepoConfig, staged: &StagedState, crate_dir: &str) -> bool {
    staged
        .deleted_paths
        .contains(&package_manifest_path(config, crate_dir))
}

fn path_belongs_to_deleted_crate(config: &RepoConfig, staged: &StagedState, path: &str) -> bool {
    package_dir_for_path(config, path)
        .is_some_and(|crate_dir| crate_manifest_deleted(config, staged, &crate_dir))
}

fn is_allowed_deleted_changelog_path(
    config: &RepoConfig,
    staged: &StagedState,
    path: &str,
) -> bool {
    package_dir_from_changelog_path(config, path)
        .is_some_and(|crate_dir| crate_manifest_deleted(config, staged, &crate_dir))
}

fn is_allowed_crate_layout_changelog_path(
    staged: &StagedState,
    config: &RepoConfig,
    path: &str,
) -> bool {
    if path == config.changelog_path {
        return true;
    }
    let Ok(layout_paths) = crate_layout_paths(config) else {
        return false;
    };
    if !is_package_changelog_path(path, &layout_paths) {
        return false;
    }
    package_dir_from_changelog_path(config, path).is_some_and(|crate_dir| {
        staged.active_crate_dirs.contains(&crate_dir)
            || crate_manifest_deleted(config, staged, &crate_dir)
    })
}

fn requires_primary_changelog(config: &RepoConfig, staged: &StagedState) -> bool {
    staged
        .paths
        .iter()
        .filter(|path| !is_changelog_path(config, path))
        .any(|path| {
            active_crate_dir_for_path(config, &staged.active_crate_dirs, path).is_none()
                && !path_belongs_to_deleted_crate(config, staged, path)
        })
}

fn validate_allowed_changelog_paths(
    config: &RepoConfig,
    staged: &StagedState,
) -> Result<(), String> {
    match config.layout {
        Layout::Root => {
            let disallowed: Vec<&String> = staged
                .changelog_paths
                .iter()
                .filter(|path| path.as_str() != config.changelog_path)
                .collect();
            if disallowed.is_empty() {
                return Ok(());
            }
            Err(format!(
                "repo-check: this repository uses a single configured changelog.\n\nOnly `{}` is allowed.\n\nDisallowed staged changelog paths:\n{}",
                config.changelog_path,
                bullet_list(disallowed.iter().map(|path| path.as_str()))
            ))
        }
        Layout::Crate => {
            let invalid: Vec<&String> = staged
                .changelog_paths
                .iter()
                .filter(|path| !is_allowed_crate_layout_changelog_path(staged, config, path))
                .collect();
            if invalid.is_empty() {
                return Ok(());
            }
            let mut details: Vec<&str> = invalid.iter().map(|value| value.as_str()).collect();
            details.sort();
            Err(format!(
                "repo-check: this repository keeps changelogs inside real crate packages, with root or governance changes owned by `{}`.\n\nDisallowed staged changelog paths:\n{}",
                config.changelog_path,
                bullet_list(details.into_iter())
            ))
        }
    }
}

fn validate_required_changelog_not_deleted(
    config: &RepoConfig,
    staged: &StagedState,
) -> Result<(), String> {
    let deleted: Vec<&str> = match config.layout {
        Layout::Root => staged
            .deleted_paths
            .iter()
            .filter(|path| path.as_str() == config.changelog_path)
            .map(|path| path.as_str())
            .collect(),
        Layout::Crate => staged
            .deleted_paths
            .iter()
            .filter(|path| {
                is_changelog_path(config, path)
                    && !is_allowed_deleted_changelog_path(config, staged, path)
            })
            .map(|path| path.as_str())
            .collect(),
    };
    if deleted.is_empty() {
        return Ok(());
    }

    Err(format!(
        "repo-check: refusing to delete active changelog files.\n\nDeleted changelog paths:\n{}",
        bullet_list(deleted.into_iter())
    ))
}

fn validate_changelog_update(
    repo_root: &Path,
    config: &RepoConfig,
    staged: &StagedState,
) -> Result<(), String> {
    match config.layout {
        Layout::Root => {
            if !root_layout_requires_changelog(config, staged) {
                return Ok(());
            }
            if staged
                .changelog_paths
                .iter()
                .any(|path| path == &config.changelog_path)
            {
                return Ok(());
            }
            Err(format!(
                "repo-check: a root-package repository must update {} in the same commit.",
                config.changelog_path
            ))
        }
        Layout::Crate => {
            let mut required_paths = BTreeSet::new();
            for crate_dir in &staged.crate_dirs_with_non_changelog_changes {
                required_paths.insert(package_changelog_path(config, crate_dir));
            }
            for crate_dir in workspace_version_inheriting_crate_dirs(repo_root, config, staged)? {
                required_paths.insert(package_changelog_path(config, &crate_dir));
            }
            if requires_primary_changelog(config, staged) {
                required_paths.insert(config.changelog_path.clone());
            }

            if required_paths.is_empty() {
                return Ok(());
            }

            let mut missing_files = Vec::new();
            let mut missing_updates = Vec::new();

            for changelog_path in required_paths {
                if !repo_root.join(&changelog_path).is_file()
                    && !staged
                        .changelog_paths
                        .iter()
                        .any(|path| path == &changelog_path)
                {
                    missing_files.push(changelog_path);
                    continue;
                }
                if !staged
                    .changelog_paths
                    .iter()
                    .any(|path| path == &changelog_path)
                {
                    missing_updates.push(changelog_path);
                }
            }

            if !missing_files.is_empty() {
                return Err(format!(
                    "repo-check: every changed crate-package or governance surface must maintain an owned changelog.\n\nCreate the missing changelog file(s):\n{}",
                    bullet_list(missing_files.iter().map(|path| path.as_str()))
                ));
            }
            if !missing_updates.is_empty() {
                return Err(format!(
                    "repo-check: every changed crate-package or governance surface must update its owned changelog.\n\nStage an [Unreleased] entry in:\n{}",
                    bullet_list(missing_updates.iter().map(|path| path.as_str()))
                ));
            }

            Ok(())
        }
    }
}

fn root_layout_requires_changelog(config: &RepoConfig, staged: &StagedState) -> bool {
    staged
        .paths
        .iter()
        .filter(|path| !is_changelog_path(config, path))
        .any(|path| !is_governance_only_root_path(path))
}

fn is_governance_only_root_path(path: &str) -> bool {
    matches!(
        path,
        "README.md" | "AGENTS.md" | "repo-check.toml" | ".gitignore"
    ) || path.starts_with("docs/")
        || path.starts_with("githooks/")
        || path.starts_with("tools/repo-check/")
        || path.starts_with(".github/")
}

fn validate_not_changelog_only(staged: &StagedState) -> Result<(), String> {
    if staged.changelog_paths.is_empty() || staged.non_changelog_count > 0 {
        return Ok(());
    }
    Err(
        "repo-check: refusing changelog-only commit; commit the actual change together with its changelog update."
            .to_string(),
    )
}

fn validate_released_sections_immutable(
    repo_root: &Path,
    config: &RepoConfig,
    staged: &StagedState,
) -> Result<(), String> {
    if env::var("OMNE_ALLOW_CHANGELOG_RELEASE_EDIT")
        .ok()
        .as_deref()
        == Some("1")
    {
        return Ok(());
    }

    let relevant_paths: Vec<&String> = match config.layout {
        Layout::Root => staged
            .changelog_paths
            .iter()
            .filter(|path| path.as_str() == config.changelog_path)
            .collect(),
        Layout::Crate => staged
            .changelog_paths
            .iter()
            .filter(|path| is_allowed_crate_layout_changelog_path(staged, config, path))
            .collect(),
    };

    for path in relevant_paths {
        let head_text = git_show_text(repo_root, &format!("HEAD:{path}"))?;
        let index_text = git_show_text(repo_root, &format!(":{path}"))?;
        let Some(head_text) = head_text else {
            continue;
        };
        let Some(index_text) = index_text else {
            continue;
        };

        if released_sections(&head_text) == released_sections(&index_text) {
            continue;
        }

        return Err(format!(
            "repo-check: refusing to modify released CHANGELOG sections.\n\nOnly edit entries under [Unreleased].\nIf you are intentionally cutting a release, re-run with:\n{}",
            override_instructions("OMNE_ALLOW_CHANGELOG_RELEASE_EDIT", "git commit ...")
        ));
    }

    Ok(())
}

fn released_sections(text: &str) -> String {
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if !found && is_released_heading(line) {
            found = true;
        }
        if found {
            lines.push(line);
        }
    }
    lines.join("\n")
}

fn is_released_heading(line: &str) -> bool {
    line.starts_with("## [")
        && line
            .chars()
            .nth(4)
            .map(|character| character.is_ascii_digit())
            .unwrap_or(false)
}

fn require_major_bump_override(repo_root: &Path, config: &RepoConfig) -> Result<(), String> {
    let changed_targets = major_change_targets(repo_root, config)?;
    if changed_targets.is_empty() {
        return Ok(());
    }
    if env::var("OMNE_ALLOW_MAJOR_VERSION_BUMP").ok().as_deref() == Some("1") {
        return Ok(());
    }

    Err(format!(
        "repo-check: refusing major version change by default.\n\nThe following targets changed their major segment:\n{}\n\nRe-run with:\n{}",
        bullet_list(changed_targets.iter().map(format_version_target)),
        override_instructions("OMNE_ALLOW_MAJOR_VERSION_BUMP", "git commit ...")
    ))
}

fn override_instructions(variable: &str, command: &str) -> String {
    format!(
        "  POSIX: {variable}=1 {command}\n  PowerShell:\n    $env:{variable} = '1'\n    {command}"
    )
}

fn require_breaking_commit_marker(
    repo_root: &Path,
    config: &RepoConfig,
    parsed: &ParsedCommitMessage,
) -> Result<(), String> {
    let changed_targets = major_change_targets(repo_root, config)?;
    if changed_targets.is_empty() || parsed.breaking {
        return Ok(());
    }

    Err(format!(
        "repo-check: major version change requires an explicit breaking commit message.\n\nTargets:\n{}\n\nDeclare the breaking change with either:\n- a `!` in the header, for example `refactor(core)!: start 1.0 transition`\n- or a `BREAKING CHANGE:` / `BREAKING-CHANGE:` footer",
        bullet_list(changed_targets.iter().map(format_version_target))
    ))
}

fn major_change_targets(
    repo_root: &Path,
    config: &RepoConfig,
) -> Result<Vec<VersionTarget>, String> {
    let targets = version_targets(repo_root, config)?;
    let mut changed = Vec::new();
    for target in targets {
        let old_major = parse_version_major(target.old_version.as_deref())?;
        let new_major = parse_version_major(target.new_version.as_deref())?;

        if new_major == Some(0) {
            continue;
        }
        if let (Some(old_major), Some(new_major)) = (old_major, new_major)
            && new_major > 0
            && new_major != old_major
        {
            changed.push(target);
        }
    }
    Ok(changed)
}

fn version_targets(repo_root: &Path, config: &RepoConfig) -> Result<Vec<VersionTarget>, String> {
    let index_config = RepoConfig::load_from_git(repo_root, "")?.unwrap_or_else(|| config.clone());
    let head_config =
        RepoConfig::load_from_git(repo_root, "HEAD")?.unwrap_or_else(|| index_config.clone());

    match (index_config.project_kind, index_config.layout) {
        (ProjectKind::Rust, Layout::Root) => {
            let head_text = git_show_text(
                repo_root,
                &format!("HEAD:{}", head_config.package_manifest_path),
            )?;
            let index_text = git_show_text(
                repo_root,
                &format!(":{}", index_config.package_manifest_path),
            )?;
            Ok(vec![VersionTarget {
                label: index_config.package_name.clone(),
                path: index_config.package_manifest_path.clone(),
                old_version: cargo_package_version(head_text.as_deref(), None).0,
                new_version: cargo_package_version(index_text.as_deref(), None).0,
            }])
        }
        (ProjectKind::Rust, Layout::Crate) => {
            let head_layout_paths = crate_layout_paths(&head_config)?;
            let index_layout_paths = crate_layout_paths(&index_config)?;
            let head_workspace = git_workspace_manifest_location(repo_root, &head_config, "HEAD")?;
            let index_workspace = git_workspace_manifest_location(repo_root, &index_config, "")?;
            let head_workspace_version =
                cargo_workspace_version(head_workspace.as_ref().map(|value| value.text.as_str()));
            let index_workspace_version =
                cargo_workspace_version(index_workspace.as_ref().map(|value| value.text.as_str()));
            let head_crate_dirs = discover_crate_dirs_from_workspace_manifest(
                repo_root,
                &head_layout_paths,
                head_workspace.as_ref().map(|value| value.text.as_str()),
                head_workspace
                    .as_ref()
                    .map(|value| value.path.as_str())
                    .unwrap_or("HEAD workspace manifest"),
            )?;
            let index_crate_dirs = discover_crate_dirs_from_workspace_manifest(
                repo_root,
                &index_layout_paths,
                index_workspace.as_ref().map(|value| value.text.as_str()),
                index_workspace
                    .as_ref()
                    .map(|value| value.path.as_str())
                    .unwrap_or("staged workspace manifest"),
            )?;

            let mut discovered = BTreeSet::new();
            discovered.extend(head_crate_dirs);
            discovered.extend(index_crate_dirs);

            let mut targets = Vec::new();
            for crate_dir in discovered {
                let head_path = package_manifest_path(&head_config, &crate_dir);
                let index_path = package_manifest_path(&index_config, &crate_dir);
                let head_text = git_show_text(repo_root, &format!("HEAD:{head_path}"))?;
                let index_text = git_show_text(repo_root, &format!(":{index_path}"))?;
                if head_text.is_none() && index_text.is_none() {
                    continue;
                }
                targets.push(VersionTarget {
                    label: crate_dir.clone(),
                    path: index_path,
                    old_version: cargo_package_version(
                        head_text.as_deref(),
                        head_workspace_version.as_deref(),
                    )
                    .0,
                    new_version: cargo_package_version(
                        index_text.as_deref(),
                        index_workspace_version.as_deref(),
                    )
                    .0,
                });
            }
            Ok(targets)
        }
        (ProjectKind::Python, Layout::Root) => {
            let head_text = git_show_text(
                repo_root,
                &format!("HEAD:{}", head_config.package_manifest_path),
            )?;
            let index_text = git_show_text(
                repo_root,
                &format!(":{}", index_config.package_manifest_path),
            )?;
            Ok(vec![VersionTarget {
                label: index_config.package_name.clone(),
                path: index_config.package_manifest_path.clone(),
                old_version: pyproject_version(head_text.as_deref()),
                new_version: pyproject_version(index_text.as_deref()),
            }])
        }
        (ProjectKind::Nodejs, Layout::Root) => {
            let head_text = git_show_text(
                repo_root,
                &format!("HEAD:{}", head_config.package_manifest_path),
            )?;
            let index_text = git_show_text(
                repo_root,
                &format!(":{}", index_config.package_manifest_path),
            )?;
            Ok(vec![VersionTarget {
                label: index_config.package_name.clone(),
                path: index_config.package_manifest_path.clone(),
                old_version: package_json_version(head_text.as_deref()),
                new_version: package_json_version(index_text.as_deref()),
            }])
        }
        _ => Ok(Vec::new()),
    }
}

fn parse_version_major(version: Option<&str>) -> Result<Option<u64>, String> {
    let Some(version) = version else {
        return Ok(None);
    };
    let version = version.trim();
    let release = version
        .rsplit_once('!')
        .map(|(_, release)| release)
        .unwrap_or(version)
        .trim();
    let release = release
        .strip_prefix('v')
        .or_else(|| release.strip_prefix('V'))
        .unwrap_or(release);
    let digits: String = release
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return Err(format!(
            "repo-check: unsupported version without numeric major segment: {version}"
        ));
    }
    let major = digits
        .parse::<u64>()
        .map_err(|_| format!("repo-check: invalid version major segment: {version}"))?;
    Ok(Some(major))
}

fn format_version_target(target: &VersionTarget) -> String {
    format!(
        "{}: {} -> {} [{}]",
        target.label,
        target.old_version.as_deref().unwrap_or("<none>"),
        target.new_version.as_deref().unwrap_or("<none>"),
        target.path
    )
}

fn cargo_workspace_version(text: Option<&str>) -> Option<String> {
    cargo_toml(text)?
        .get("workspace")?
        .get("package")?
        .get("version")?
        .as_str()
        .map(|value| value.to_string())
}

fn cargo_package_version(
    text: Option<&str>,
    workspace_version: Option<&str>,
) -> (Option<String>, bool) {
    let Some(parsed) = cargo_toml(text) else {
        return (None, false);
    };
    let Some(package) = parsed.get("package") else {
        return (None, false);
    };
    let Some(version) = package.get("version") else {
        return (None, false);
    };
    if let Some(version) = version.as_str() {
        return (Some(version.to_string()), false);
    }
    if version
        .as_table()
        .and_then(|table| table.get("workspace"))
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        return (workspace_version.map(|value| value.to_string()), true);
    }
    (None, false)
}

fn cargo_section_value(text: Option<&str>, section: &str, key: &str) -> Option<String> {
    let mut current = cargo_toml(text)?;
    for segment in section.split('.') {
        current = current.get(segment)?.clone();
    }
    current.get(key)?.as_str().map(|value| value.to_string())
}

fn cargo_toml(text: Option<&str>) -> Option<TomlValue> {
    toml::from_str(text?).ok()
}

fn pyproject_version(text: Option<&str>) -> Option<String> {
    cargo_section_value(text, "project", "version")
}

fn pyproject_requires_python(text: Option<&str>) -> Option<String> {
    cargo_section_value(text, "project", "requires-python")
}

fn package_json_version(text: Option<&str>) -> Option<String> {
    serde_json::from_str::<JsonValue>(text?)
        .ok()?
        .get("version")?
        .as_str()
        .map(|value| value.to_string())
}

fn discover_crate_dirs(
    repo_root: &Path,
    layout_paths: &CrateLayoutPaths,
    workspace_manifest_path: &str,
) -> Result<Vec<String>, String> {
    let manifest_path = repo_root.join(workspace_manifest_path);
    let manifest = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "repo-check: failed to read {}: {error}",
            manifest_path.display()
        )
    })?;
    discover_crate_dirs_from_workspace_manifest(
        repo_root,
        layout_paths,
        Some(manifest.as_str()),
        workspace_manifest_path,
    )
}

fn staged_active_crate_dirs(
    repo_root: &Path,
    config: &RepoConfig,
) -> Result<BTreeSet<String>, String> {
    if config.layout != Layout::Crate || config.project_kind != ProjectKind::Rust {
        return Ok(BTreeSet::new());
    }
    let layout_paths = crate_layout_paths(config)?;
    let workspace_manifest = git_workspace_manifest_location(repo_root, config, "")?;
    Ok(discover_crate_dirs_from_workspace_manifest(
        repo_root,
        &layout_paths,
        workspace_manifest.as_ref().map(|value| value.text.as_str()),
        workspace_manifest
            .as_ref()
            .map(|value| value.path.as_str())
            .unwrap_or("staged workspace manifest"),
    )?
    .into_iter()
    .collect())
}

fn discover_crate_dirs_from_workspace_manifest(
    repo_root: &Path,
    layout_paths: &CrateLayoutPaths,
    text: Option<&str>,
    source_label: &str,
) -> Result<Vec<String>, String> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let parsed = cargo_toml(Some(text)).ok_or_else(|| {
        format!("repo-check: failed to parse workspace manifest from {source_label}")
    })?;
    let members = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(|members| members.as_array())
        .cloned()
        .unwrap_or_default();
    let excludes = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("exclude"))
        .and_then(|members| members.as_array())
        .cloned()
        .unwrap_or_default();
    let workspace_manifest_dir = workspace_manifest_parent(source_label);

    let mut crate_dirs = BTreeSet::new();
    for member in members {
        let Some(member) = member.as_str() else {
            continue;
        };
        update_workspace_crate_dirs(
            repo_root,
            layout_paths,
            &workspace_manifest_dir,
            member,
            true,
            &mut crate_dirs,
        )?;
    }
    for exclude in excludes {
        let Some(exclude) = exclude.as_str() else {
            continue;
        };
        update_workspace_crate_dirs(
            repo_root,
            layout_paths,
            &workspace_manifest_dir,
            exclude,
            false,
            &mut crate_dirs,
        )?;
    }
    Ok(crate_dirs.into_iter().collect())
}

fn update_workspace_crate_dirs(
    repo_root: &Path,
    layout_paths: &CrateLayoutPaths,
    workspace_manifest_dir: &Path,
    member: &str,
    insert: bool,
    crate_dirs: &mut BTreeSet<String>,
) -> Result<(), String> {
    let container = layout_paths
        .container_dir
        .to_string_lossy()
        .replace('\\', "/");
    let normalized = normalize_workspace_member(member);
    let resolved = normalize_repo_relative_path(&workspace_manifest_dir.join(&normalized));
    let wildcard = format!("{container}/*");

    if resolved == wildcard {
        let crates_dir = repo_root.join(&layout_paths.container_dir);
        if !crates_dir.is_dir() {
            return Ok(());
        }
        for entry in fs::read_dir(&crates_dir).map_err(|error| {
            format!(
                "repo-check: failed to read {}: {error}",
                crates_dir.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "repo-check: failed to read {} entry: {error}",
                    crates_dir.display()
                )
            })?;
            let path = entry.path();
            if !path.is_dir() || !path.join(&layout_paths.manifest_name).is_file() {
                continue;
            }
            let Some(crate_dir) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            update_crate_dir_set(crate_dirs, crate_dir, insert);
        }
        return Ok(());
    }

    let candidate = Path::new(&resolved);
    let Ok(relative) = candidate.strip_prefix(&layout_paths.container_dir) else {
        return Ok(());
    };
    let mut parts = relative.components();
    let Some(crate_dir) = parts.next().and_then(|part| part.as_os_str().to_str()) else {
        return Ok(());
    };
    let remainder: Vec<_> = parts.collect();
    let matches_member = remainder.is_empty()
        || (remainder.len() == 1
            && remainder[0].as_os_str() == std::ffi::OsStr::new(&layout_paths.manifest_name));
    if !matches_member {
        return Ok(());
    }
    if repo_root
        .join(package_manifest_path_with_layout(layout_paths, crate_dir))
        .is_file()
    {
        update_crate_dir_set(crate_dirs, crate_dir, insert);
    }
    Ok(())
}

fn normalize_workspace_member(member: &str) -> String {
    member
        .trim()
        .trim_start_matches("./")
        .trim_end_matches('/')
        .replace('\\', "/")
}

fn update_crate_dir_set(crate_dirs: &mut BTreeSet<String>, crate_dir: &str, insert: bool) {
    if insert {
        crate_dirs.insert(crate_dir.to_string());
    } else {
        crate_dirs.remove(crate_dir);
    }
}

fn workspace_version_inheriting_crate_dirs(
    repo_root: &Path,
    config: &RepoConfig,
    staged: &StagedState,
) -> Result<BTreeSet<String>, String> {
    let head_workspace = git_workspace_manifest_location(repo_root, config, "HEAD")?;
    let index_workspace = git_workspace_manifest_location(repo_root, config, "")?;
    let Some(index_workspace) = index_workspace else {
        return Ok(BTreeSet::new());
    };

    if !staged
        .paths
        .iter()
        .any(|path| path == &index_workspace.path)
    {
        return Ok(BTreeSet::new());
    }

    if cargo_workspace_version(head_workspace.as_ref().map(|value| value.text.as_str()))
        == cargo_workspace_version(Some(index_workspace.text.as_str()))
    {
        return Ok(BTreeSet::new());
    }

    let head_workspace_version =
        cargo_workspace_version(head_workspace.as_ref().map(|value| value.text.as_str()));
    let index_workspace_version = cargo_workspace_version(Some(index_workspace.text.as_str()));
    let mut crate_dirs = BTreeSet::new();
    let layout_paths = crate_layout_paths(config)?;

    let head_crate_dirs = discover_crate_dirs_from_workspace_manifest(
        repo_root,
        &layout_paths,
        head_workspace.as_ref().map(|value| value.text.as_str()),
        head_workspace
            .as_ref()
            .map(|value| value.path.as_str())
            .unwrap_or("HEAD workspace manifest"),
    )?;
    let index_crate_dirs = discover_crate_dirs_from_workspace_manifest(
        repo_root,
        &layout_paths,
        Some(index_workspace.text.as_str()),
        &index_workspace.path,
    )?;

    let mut discovered = BTreeSet::new();
    discovered.extend(head_crate_dirs);
    discovered.extend(index_crate_dirs);

    for crate_dir in discovered {
        let path = package_manifest_path(config, &crate_dir);
        let head_text = git_show_text(repo_root, &format!("HEAD:{path}"))?;
        let index_text = git_show_text(repo_root, &format!(":{path}"))?;
        let head_inherits =
            cargo_package_version(head_text.as_deref(), head_workspace_version.as_deref()).1;
        let index_inherits =
            cargo_package_version(index_text.as_deref(), index_workspace_version.as_deref()).1;
        if head_inherits || index_inherits {
            crate_dirs.insert(crate_dir);
        }
    }

    Ok(crate_dirs)
}

fn run_workspace_checks_on_staged_snapshot(
    repo_root: &Path,
    config: &RepoConfig,
    mode: WorkspaceMode,
) -> Result<(), String> {
    let snapshot = TempDir::new("repo-check-index")?;
    export_index_snapshot(repo_root, snapshot.path())?;
    run_workspace_checks(snapshot.path(), snapshot.path(), config, mode)
}

fn run_workspace_checks_on_worktree_snapshot(
    repo_root: &Path,
    config: &RepoConfig,
    mode: WorkspaceMode,
) -> Result<(), String> {
    let snapshot = TempDir::new("repo-check-worktree")?;
    copy_worktree_snapshot(repo_root, snapshot.path())?;
    run_workspace_checks(snapshot.path(), snapshot.path(), config, mode)
}

fn export_index_snapshot(repo_root: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "repo-check: failed to create staged snapshot directory {}: {error}",
            destination.display()
        )
    })?;
    let mut prefix = destination.as_os_str().to_os_string();
    prefix.push(std::path::MAIN_SEPARATOR_STR);
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("checkout-index")
        .arg("--all")
        .arg("--prefix")
        .arg(&prefix)
        .output()
        .map_err(|error| {
            format!(
                "repo-check: failed to export staged snapshot from {}: {error}",
                repo_root.display()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "repo-check: git command failed: git -C {} checkout-index --all --prefix {:?}\n\n{}",
        repo_root.display(),
        prefix,
        git_output_detail(&output)
    ))
}

fn copy_worktree_snapshot(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "repo-check: failed to create worktree snapshot directory {}: {error}",
            destination.display()
        )
    })?;
    copy_directory_entries(source, destination)
}

fn copy_directory_entries(source: &Path, destination: &Path) -> Result<(), String> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| format!("repo-check: failed to read {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "repo-check: failed to read {} entry: {error}",
                source.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = entry.file_type().map_err(|error| {
            format!(
                "repo-check: failed to inspect {}: {error}",
                source_path.display()
            )
        })?;
        if metadata.is_dir() {
            if should_skip_snapshot_dir(&source_path) {
                continue;
            }
            fs::create_dir_all(&destination_path).map_err(|error| {
                format!(
                    "repo-check: failed to create {}: {error}",
                    destination_path.display()
                )
            })?;
            copy_directory_entries(&source_path, &destination_path)?;
            continue;
        }
        if metadata.is_symlink() {
            copy_symlink_entry(&source_path, &destination_path)?;
            continue;
        }
        if metadata.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "repo-check: failed to copy {} -> {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn copy_symlink_entry(source: &Path, destination: &Path) -> Result<(), String> {
    let link_target = fs::read_link(source).map_err(|error| {
        format!(
            "repo-check: failed to read symlink {}: {error}",
            source.display()
        )
    })?;
    create_symlink_snapshot_entry(source, destination, &link_target)
}

#[cfg(unix)]
fn create_symlink_snapshot_entry(
    source: &Path,
    destination: &Path,
    link_target: &Path,
) -> Result<(), String> {
    std::os::unix::fs::symlink(link_target, destination).map_err(|error| {
        format!(
            "repo-check: failed to recreate symlink {} -> {} from {}: {error}",
            destination.display(),
            link_target.display(),
            source.display()
        )
    })
}

#[cfg(windows)]
fn create_symlink_snapshot_entry(
    source: &Path,
    destination: &Path,
    link_target: &Path,
) -> Result<(), String> {
    let target_metadata = fs::metadata(source).map_err(|error| {
        format!(
            "repo-check: failed to inspect symlink target for {}: {error}",
            source.display()
        )
    })?;
    let create = if target_metadata.is_dir() {
        std::os::windows::fs::symlink_dir
    } else {
        std::os::windows::fs::symlink_file
    };
    create(link_target, destination).map_err(|error| {
        format!(
            "repo-check: failed to recreate symlink {} -> {} from {}: {error}",
            destination.display(),
            link_target.display(),
            source.display()
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn create_symlink_snapshot_entry(
    source: &Path,
    destination: &Path,
    link_target: &Path,
) -> Result<(), String> {
    let _ = (source, destination, link_target);
    Err("repo-check: worktree snapshot does not support symlinks on this platform".to_string())
}

fn should_skip_snapshot_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| WORKTREE_SNAPSHOT_IGNORED_DIRS.contains(&name))
}

fn read_commit_message(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|error| format!("repo-check: failed to read {}: {error}", path.display()))
}

fn is_special_commit_message(line: &str) -> bool {
    line.starts_with("Merge ")
        || line.starts_with("Revert \"")
        || line.starts_with("fixup! ")
        || line.starts_with("squash! ")
}

fn parse_conventional_commit(message: &str) -> Result<ParsedCommitMessage, String> {
    let line = message
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r');
    let (head, subject) = line
        .split_once(": ")
        .ok_or_else(|| conventional_commit_error(line))?;
    if subject.trim().is_empty() {
        return Err(conventional_commit_error(line));
    }

    let mut head = head;
    let mut breaking = false;
    if head.ends_with('!') {
        breaking = true;
        head = &head[..head.len() - 1];
    }

    let commit_type = if let Some(scope_start) = head.find('(') {
        if !head.ends_with(')') {
            return Err(conventional_commit_error(line));
        }
        let commit_type = &head[..scope_start];
        let scope = &head[scope_start + 1..head.len() - 1];
        if scope.is_empty() || !scope.chars().all(is_valid_scope_character) {
            return Err(conventional_commit_error(line));
        }
        commit_type
    } else {
        head
    };

    if !ALLOWED_COMMIT_TYPES.contains(&commit_type) {
        return Err(conventional_commit_error(line));
    }

    Ok(ParsedCommitMessage {
        breaking: breaking || has_breaking_footer(message),
    })
}

fn has_breaking_footer(message: &str) -> bool {
    let mut previous_blank = false;
    for line in message.lines().skip(1) {
        let line = line.trim_end_matches('\r');
        if previous_blank
            && (line.starts_with("BREAKING CHANGE:") || line.starts_with("BREAKING-CHANGE:"))
        {
            return true;
        }
        previous_blank = line.trim().is_empty();
    }
    false
}

fn is_valid_scope_character(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || matches!(character, '.' | '_' | '-')
}

fn conventional_commit_error(line: &str) -> String {
    format!(
        "repo-check: invalid commit message.\n\nExpected Conventional Commits format:\n  <type>(<scope>)!: <subject>\n\nAllowed types:\n- {}\n\nGot: {}",
        ALLOWED_COMMIT_TYPES.join("\n- "),
        line
    )
}

fn run_workspace_checks(
    repo_root: &Path,
    target_root_key: &Path,
    config: &RepoConfig,
    mode: WorkspaceMode,
) -> Result<(), String> {
    eprintln!(
        "repo-check: running {:?} checks for {} ({:?}, template {}, schema {}, manifest {}, changelog {}, source {})",
        mode,
        config.repo_name,
        config.project_kind,
        config.template_version,
        config.schema_version,
        config.package_manifest_path,
        config.changelog_path,
        config.primary_source_path
    );
    if config.layout == Layout::Crate {
        eprintln!("repo-check: primary crate dir {}", config.crate_dir);
    }

    match config.project_kind {
        ProjectKind::Rust => {
            let cargo_target_dir = shared_cargo_target_dir(target_root_key)?;
            run_named_command(
                repo_root,
                "rust fmt",
                "cargo",
                &["fmt", "--all", "--", "--check"],
                Some(cargo_target_dir.as_path()),
            )?;
            run_named_command(
                repo_root,
                "rust check",
                "cargo",
                &["check", "--workspace", "--all-targets", "--all-features"],
                Some(cargo_target_dir.as_path()),
            )?;
            run_named_command(
                repo_root,
                "rust test",
                "cargo",
                &["test", "--workspace", "--all-features"],
                Some(cargo_target_dir.as_path()),
            )?;
            run_named_command(
                repo_root,
                "rust clippy",
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--",
                    "-D",
                    "warnings",
                ],
                Some(cargo_target_dir.as_path()),
            )?;
            Ok(())
        }
        ProjectKind::Python => {
            let manifest_path = repo_root.join(&config.package_manifest_path);
            let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
                format!(
                    "repo-check: failed to read {}: {error}",
                    manifest_path.display()
                )
            })?;
            let requires_python =
                pyproject_requires_python(Some(&manifest_text)).ok_or_else(|| {
                    format!(
                        "repo-check: {} must declare [project].requires-python",
                        config.package_manifest_path
                    )
                })?;
            validate_python_requires_python_floor(&requires_python, &config.package_manifest_path)?;
            let python =
                detect_python_runtime(repo_root, &requires_python, &config.package_manifest_path)?;
            run_prefixed_command(
                repo_root,
                "python compileall",
                &python.command,
                &["-m", "compileall", &config.python_package, "tests"],
            )?;
            run_prefixed_command(
                repo_root,
                "python unittest",
                &python.command,
                &[
                    "-m", "unittest", "discover", "-s", "tests", "-p", "test*.py",
                ],
            )
        }
        ProjectKind::Nodejs => {
            ensure_command_available(repo_root, "node", &["--version"])?;
            run_named_command(
                repo_root,
                "node syntax",
                "node",
                &["--check", &config.primary_source_path],
                None,
            )?;
            run_named_command(repo_root, "node test", "node", &["--test"], None)
        }
    }
}

fn shared_cargo_target_dir(target_root_key: &Path) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("CARGO_TARGET_DIR") {
        let path = PathBuf::from(path);
        fs::create_dir_all(&path).map_err(|error| {
            format!(
                "repo-check: failed to create CARGO_TARGET_DIR {}: {error}",
                path.display()
            )
        })?;
        return Ok(path);
    }

    let path = env::temp_dir().join(shared_cargo_target_dir_name(target_root_key));
    fs::create_dir_all(&path).map_err(|error| {
        format!(
            "repo-check: failed to create shared cargo target dir {}: {error}",
            path.display()
        )
    })?;
    Ok(path)
}

fn shared_cargo_target_dir_name(target_root_key: &Path) -> String {
    let normalized = target_root_key
        .canonicalize()
        .unwrap_or_else(|_| target_root_key.to_path_buf());
    let normalized = normalize_repo_relative_path(&normalized);

    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let hash = hasher.finish();

    let basename = target_root_key
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| {
            value
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                        character
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
                .trim_matches('-')
                .to_string()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "repo".to_string());

    format!("omne-repo-check-target-{basename}-{hash:016x}")
}

fn detect_python_runtime(
    repo_root: &Path,
    requires_python: &str,
    manifest_path: &str,
) -> Result<PythonRuntime, String> {
    let candidates = [
        vec!["python".to_string()],
        vec!["python3".to_string()],
        vec!["py".to_string(), "-3".to_string()],
    ];
    let mut detected = Vec::new();

    for candidate in candidates {
        let Some(version) = probe_python_version(repo_root, &candidate) else {
            continue;
        };
        if python_requirement_matches(requires_python, version)? {
            return Ok(PythonRuntime { command: candidate });
        }
        detected.push(format!(
            "`{}` -> {}",
            render_command_prefix(&candidate),
            version
        ));
    }

    if detected.is_empty() {
        return Err(
            "repo-check: unable to locate a Python interpreter. Tried `python`, `python3`, and `py -3`."
                .to_string(),
        );
    }

    Err(format!(
        "repo-check: no available Python interpreter satisfies `{}` from {}.\n\nDetected interpreters:\n{}",
        requires_python,
        manifest_path,
        bullet_list(detected.iter().map(String::as_str))
    ))
}

fn validate_python_requires_python_floor(
    requires_python: &str,
    manifest_path: &str,
) -> Result<(), String> {
    if python_requirement_meets_template_floor(requires_python)? {
        return Ok(());
    }

    Err(format!(
        "repo-check: python project must declare `project.requires-python` compatible with >=3.11 in {}.\n\nFound: {}",
        manifest_path, requires_python
    ))
}

fn probe_python_version(repo_root: &Path, prefix: &[String]) -> Option<PythonVersion> {
    let program = prefix.first()?;
    let mut command = Command::new(program);
    command.current_dir(repo_root);
    if prefix.len() > 1 {
        command.args(&prefix[1..]);
    }
    command.args([
        "-c",
        "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}.{sys.version_info[2]}')",
    ]);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_python_version(stdout.trim()).ok()
}

fn render_command_prefix(prefix: &[String]) -> String {
    prefix.join(" ")
}

fn python_requirement_matches(
    requires_python: &str,
    version: PythonVersion,
) -> Result<bool, String> {
    let mut saw_clause = false;
    for raw_clause in requires_python.split(',') {
        let clause = raw_clause.trim();
        if clause.is_empty() {
            continue;
        }
        saw_clause = true;
        if !python_requirement_clause_matches(clause, version)? {
            return Ok(false);
        }
    }
    if saw_clause {
        Ok(true)
    } else {
        Err("repo-check: requires-python must not be empty".to_string())
    }
}

fn python_requirement_meets_template_floor(requires_python: &str) -> Result<bool, String> {
    for disallowed in [
        PythonVersion {
            major: 3,
            minor: 9,
            patch: 0,
        },
        PythonVersion {
            major: 3,
            minor: 10,
            patch: 0,
        },
    ] {
        if python_requirement_matches(requires_python, disallowed)? {
            return Ok(false);
        }
    }

    for allowed in [
        PythonVersion {
            major: 3,
            minor: 11,
            patch: 0,
        },
        PythonVersion {
            major: 3,
            minor: 12,
            patch: 0,
        },
        PythonVersion {
            major: 4,
            minor: 0,
            patch: 0,
        },
        PythonVersion {
            major: 99,
            minor: 0,
            patch: 0,
        },
    ] {
        if python_requirement_matches(requires_python, allowed)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn python_requirement_clause_matches(clause: &str, version: PythonVersion) -> Result<bool, String> {
    for operator in [">=", "<=", "==", "!=", "~=", ">", "<"] {
        if let Some(raw_version) = clause.strip_prefix(operator) {
            return python_requirement_operator_matches(operator, raw_version.trim(), version);
        }
    }
    Err(format!(
        "repo-check: unsupported requires-python clause: {clause}"
    ))
}

fn python_requirement_operator_matches(
    operator: &str,
    raw_version: &str,
    version: PythonVersion,
) -> Result<bool, String> {
    let wildcard = raw_version.ends_with(".*");
    let raw_version = raw_version.trim_end_matches(".*").trim();
    let (target, segments) = parse_python_version_with_segments(raw_version)?;
    match operator {
        ">=" => Ok(version >= target),
        "<=" => Ok(version <= target),
        ">" => Ok(version > target),
        "<" => Ok(version < target),
        "==" if wildcard => Ok(python_version_prefix_matches(version, target, segments)),
        "!=" if wildcard => Ok(!python_version_prefix_matches(version, target, segments)),
        "==" => Ok(version == target),
        "!=" => Ok(version != target),
        "~=" => {
            if segments < 2 {
                return Err(format!(
                    "repo-check: unsupported requires-python compatible release clause: {operator}{raw_version}"
                ));
            }
            Ok(version >= target && version < python_compatible_upper_bound(target, segments))
        }
        _ => Err(format!(
            "repo-check: unsupported requires-python clause: {operator}{raw_version}"
        )),
    }
}

fn python_compatible_upper_bound(version: PythonVersion, segments: usize) -> PythonVersion {
    if segments <= 2 {
        return PythonVersion {
            major: version.major + 1,
            minor: 0,
            patch: 0,
        };
    }
    PythonVersion {
        major: version.major,
        minor: version.minor + 1,
        patch: 0,
    }
}

fn python_version_prefix_matches(
    version: PythonVersion,
    prefix: PythonVersion,
    segments: usize,
) -> bool {
    match segments {
        1 => version.major == prefix.major,
        2 => version.major == prefix.major && version.minor == prefix.minor,
        _ => version == prefix,
    }
}

fn parse_python_version(text: &str) -> Result<PythonVersion, String> {
    parse_python_version_with_segments(text).map(|(version, _)| version)
}

fn parse_python_version_with_segments(text: &str) -> Result<(PythonVersion, usize), String> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(format!("repo-check: invalid Python version: {text}"));
    }

    let mut numbers = [0_u64; 3];
    for (index, part) in parts.iter().enumerate() {
        let digits: String = part
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        if digits.is_empty() {
            return Err(format!("repo-check: invalid Python version: {text}"));
        }
        numbers[index] = digits
            .parse::<u64>()
            .map_err(|_| format!("repo-check: invalid Python version: {text}"))?;
    }

    Ok((
        PythonVersion {
            major: numbers[0],
            minor: numbers[1],
            patch: numbers[2],
        },
        parts.len(),
    ))
}

fn ensure_command_available(
    repo_root: &Path,
    program: &str,
    probe_args: &[&str],
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.current_dir(repo_root).args(probe_args);
    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => Err(format!(
            "repo-check: `{program}` is installed but did not respond to the probe command."
        )),
        Err(error) => Err(format!(
            "repo-check: failed to execute `{program}`: {error}"
        )),
    }
}

fn run_named_command(
    repo_root: &Path,
    label: &str,
    program: &str,
    args: &[&str],
    cargo_target_dir: Option<&Path>,
) -> Result<(), String> {
    eprintln!("repo-check: {label}");
    let mut command = Command::new(program);
    command.current_dir(repo_root).args(args);
    maybe_set_cargo_target_dir(&mut command, program, cargo_target_dir);
    run_command_checked(label, &mut command)
}

fn run_prefixed_command(
    repo_root: &Path,
    label: &str,
    prefix: &[String],
    args: &[&str],
) -> Result<(), String> {
    let Some(program) = prefix.first() else {
        return Err(format!("repo-check: missing command prefix for {label}"));
    };
    eprintln!("repo-check: {label}");
    let mut command = Command::new(program);
    command.current_dir(repo_root);
    if prefix.len() > 1 {
        command.args(&prefix[1..]);
    }
    command.args(args);
    run_command_checked(label, &mut command)
}

fn maybe_set_cargo_target_dir(
    command: &mut Command,
    program: &str,
    cargo_target_dir: Option<&Path>,
) {
    if program == "cargo"
        && let Some(path) = cargo_target_dir
    {
        command.env("CARGO_TARGET_DIR", path);
    }
}

fn run_command_checked(label: &str, command: &mut Command) -> Result<(), String> {
    let rendered = format!("{command:?}");
    let output = command
        .output()
        .map_err(|error| format!("repo-check: failed to execute {rendered}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("command failed while running {label}")
    };
    Err(format!("{detail}\n\ncommand: {rendered}"))
}

fn git_output(repo_root: &Path, args: &[&str], allow_failure: bool) -> Result<String, String> {
    let output = run_git(repo_root, args)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    if allow_failure {
        return Ok(String::new());
    }

    Err(render_git_failure(repo_root, args, &output))
}

fn git_show_text(repo_root: &Path, spec: &str) -> Result<Option<String>, String> {
    match parse_git_show_spec(spec) {
        GitShowSpec::HeadPath(path) if !git_head_path_exists(repo_root, path)? => return Ok(None),
        GitShowSpec::IndexPath(path) if !git_index_path_exists(repo_root, path)? => {
            return Ok(None);
        }
        _ => {}
    }

    let output = run_git(repo_root, &["show", spec])?;
    if output.status.success() {
        return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
    }
    Err(render_git_failure(repo_root, &["show", spec], &output))
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(args);
    command
        .output()
        .map_err(|error| format!("repo-check: failed to execute git {:?}: {error}", args))
}

fn render_git_failure(repo_root: &Path, args: &[&str], output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    format!(
        "repo-check: git command failed: git -C {} {}\n\n{}",
        repo_root.display(),
        args.join(" "),
        detail
    )
}

enum GitShowSpec<'a> {
    HeadPath(&'a str),
    IndexPath(&'a str),
    Other,
}

fn parse_git_show_spec(spec: &str) -> GitShowSpec<'_> {
    if let Some(path) = spec.strip_prefix("HEAD:") {
        return GitShowSpec::HeadPath(path);
    }
    if let Some(path) = spec.strip_prefix(':') {
        return GitShowSpec::IndexPath(path);
    }
    GitShowSpec::Other
}

fn git_head_path_exists(repo_root: &Path, path: &str) -> Result<bool, String> {
    let head_output = run_git(repo_root, &["rev-parse", "--verify", "HEAD"])?;
    if !head_output.status.success() {
        if git_missing_head(&head_output) {
            return Ok(false);
        }
        return Err(render_git_failure(
            repo_root,
            &["rev-parse", "--verify", "HEAD"],
            &head_output,
        ));
    }

    let object_spec = format!("HEAD:{path}");
    let output = run_git(repo_root, &["cat-file", "-e", &object_spec])?;
    if output.status.success() {
        return Ok(true);
    }
    if git_missing_head_path(&output) {
        return Ok(false);
    }
    Err(render_git_failure(
        repo_root,
        &["cat-file", "-e", &object_spec],
        &output,
    ))
}

fn git_index_path_exists(repo_root: &Path, path: &str) -> Result<bool, String> {
    let output = run_git(repo_root, &["ls-files", "--error-unmatch", "--", path])?;
    if output.status.success() {
        return Ok(true);
    }
    if git_missing_index_path(&output) {
        return Ok(false);
    }
    Err(render_git_failure(
        repo_root,
        &["ls-files", "--error-unmatch", "--", path],
        &output,
    ))
}

fn git_missing_head(output: &Output) -> bool {
    let detail = git_output_detail(output);
    detail.contains("Needed a single revision") || detail.contains("invalid object name 'HEAD'")
}

fn git_missing_head_path(output: &Output) -> bool {
    let detail = git_output_detail(output);
    detail.contains("Not a valid object name HEAD:")
        || detail.contains("does not exist in 'HEAD'")
        || detail.contains("exists on disk, but not in 'HEAD'")
}

fn git_missing_index_path(output: &Output) -> bool {
    git_output_detail(output).contains("did not match any file(s) known to git")
}

fn git_output_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn bullet_list(values: impl Iterator<Item = impl AsRef<str>>) -> String {
    values
        .map(|value| format!("- {}", value.as_ref()))
        .collect::<Vec<_>>()
        .join("\n")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self, String> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("repo-check: invalid system clock: {error}"))?
            .as_nanos();
        let path = env::temp_dir().join(format!("repo-check-{prefix}-{nanos}-{unique}"));
        fs::create_dir_all(&path).map_err(|error| {
            format!(
                "repo-check: failed to create temp dir {}: {error}",
                path.display()
            )
        })?;
        Ok(Self { path })
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

impl std::fmt::Display for PythonVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn git_show_text_returns_none_for_missing_head_and_index_paths() {
        let sandbox = TempDir::new("git-show-missing");
        run_ok(
            sandbox.path(),
            &["init", "-b", "main"],
            "failed to initialize git repo",
        );
        run_ok(
            sandbox.path(),
            &["config", "user.name", "Smoke Test"],
            "failed to configure git user.name",
        );
        run_ok(
            sandbox.path(),
            &["config", "user.email", "smoke@example.com"],
            "failed to configure git user.email",
        );

        fs::write(sandbox.path().join("tracked.txt"), "tracked\n").expect("write tracked file");
        run_ok(
            sandbox.path(),
            &["add", "tracked.txt"],
            "failed to stage tracked file",
        );
        run_ok(
            sandbox.path(),
            &["commit", "-m", "feat(repo): initial"],
            "failed to create initial commit",
        );

        assert_eq!(
            git_show_text(sandbox.path(), "HEAD:missing.txt").expect("head lookup should succeed"),
            None
        );
        assert_eq!(
            git_show_text(sandbox.path(), ":missing.txt").expect("index lookup should succeed"),
            None
        );
    }

    #[test]
    fn git_show_text_reports_non_repo_git_errors() {
        let sandbox = TempDir::new("git-show-error");
        let error = git_show_text(sandbox.path(), "HEAD:missing.txt")
            .expect_err("non-git directory should fail");
        assert!(
            error.contains("git command failed"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("rev-parse --verify HEAD"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn python_requirement_matches_detects_supported_versions() {
        assert!(
            python_requirement_matches(
                ">=3.11,<4",
                PythonVersion {
                    major: 3,
                    minor: 11,
                    patch: 2,
                },
            )
            .expect("evaluate python requirement"),
        );
        assert!(
            !python_requirement_matches(
                ">=3.11,<4",
                PythonVersion {
                    major: 3,
                    minor: 10,
                    patch: 12,
                },
            )
            .expect("evaluate python requirement"),
        );
    }

    #[test]
    fn python_requirement_matches_supports_compatible_release_and_wildcards() {
        assert!(
            python_requirement_matches(
                "~=3.11",
                PythonVersion {
                    major: 3,
                    minor: 12,
                    patch: 0,
                },
            )
            .expect("evaluate compatible release"),
        );
        assert!(
            python_requirement_matches(
                "==3.11.*",
                PythonVersion {
                    major: 3,
                    minor: 11,
                    patch: 9,
                },
            )
            .expect("evaluate wildcard clause"),
        );
        assert!(
            !python_requirement_matches(
                "==3.11.*",
                PythonVersion {
                    major: 3,
                    minor: 12,
                    patch: 0,
                },
            )
            .expect("evaluate wildcard clause"),
        );
    }

    #[test]
    fn python_requirement_meets_template_floor_rejects_lower_contracts() {
        assert!(
            !python_requirement_meets_template_floor(">=3.10").expect("evaluate lower floor"),
            ">=3.10 should be rejected because it admits Python 3.10"
        );
        assert!(
            !python_requirement_meets_template_floor("!=3.10.*")
                .expect("evaluate exclusion-only floor"),
            "an exclusion-only spec should not satisfy the template floor"
        );
    }

    #[test]
    fn python_requirement_meets_template_floor_accepts_3_11_or_higher() {
        assert!(
            python_requirement_meets_template_floor(">=3.11,<4").expect("evaluate template floor"),
            ">=3.11 should satisfy the template floor"
        );
        assert!(
            python_requirement_meets_template_floor(">3.11").expect("evaluate stricter floor"),
            "a stricter floor above 3.11 should still satisfy the template floor"
        );
    }

    #[test]
    fn parse_version_major_accepts_prerelease_build_metadata_and_pep440_forms() {
        assert_eq!(
            parse_version_major(Some("1.2.3-alpha.1+build.5")).expect("parse semver prerelease"),
            Some(1)
        );
        assert_eq!(
            parse_version_major(Some("v2.0.0-beta.1")).expect("parse leading-v version"),
            Some(2)
        );
        assert_eq!(
            parse_version_major(Some("1!3.0.0rc1+local")).expect("parse pep440 epoch/local"),
            Some(3)
        );
        assert_eq!(
            parse_version_major(Some(" 4.1.0.post2 ")).expect("parse padded pep440 version"),
            Some(4)
        );
    }

    #[test]
    fn validate_package_manifest_shape_requires_real_manifest_fields() {
        let repo = TempDir::new("manifest-shape");
        let fake_node_manifest = repo.path().join("package.json");
        fs::write(&fake_node_manifest, "{\"name\":\"shape-node\"}\n")
            .expect("write fake node manifest");
        let fake_rust_manifest = repo.path().join("Cargo.toml");
        fs::write(
            &fake_rust_manifest,
            "[package]\nname = \"shape-rust\"\nedition = \"2024\"\n",
        )
        .expect("write fake rust manifest");

        let node_config = RepoConfig {
            template_version: "1".to_string(),
            schema_version: REPO_CHECK_SCHEMA_VERSION.to_string(),
            repo_name: "shape-node".to_string(),
            project_kind: ProjectKind::Nodejs,
            layout: Layout::Root,
            package_name: "shape-node".to_string(),
            crate_dir: "shape-node".to_string(),
            python_package: "shape_node".to_string(),
            package_manifest_path: "package.json".to_string(),
            changelog_path: "CHANGELOG.md".to_string(),
            primary_source_path: "src/index.js".to_string(),
        };
        let node_error = validate_package_manifest_shape(&fake_node_manifest, &node_config)
            .expect_err("invalid node manifest should fail");
        assert!(
            node_error.contains("package.json with top-level string `name` and `version` fields"),
            "unexpected node manifest error: {node_error}"
        );

        let rust_config = RepoConfig {
            project_kind: ProjectKind::Rust,
            package_manifest_path: "Cargo.toml".to_string(),
            ..node_config.clone()
        };
        let rust_error = validate_package_manifest_shape(&fake_rust_manifest, &rust_config)
            .expect_err("invalid rust manifest should fail");
        assert!(
            rust_error.contains("Cargo package manifest with `package.name` and version fields"),
            "unexpected rust manifest error: {rust_error}"
        );
    }

    #[test]
    fn validate_changelog_shape_requires_unreleased_section() {
        let repo = TempDir::new("changelog-shape");
        let changelog = repo.path().join("CHANGELOG.md");
        fs::write(&changelog, "# Changelog\n\n## [0.1.0] - 2026-01-01\n")
            .expect("write fake changelog");

        let error = validate_changelog_shape(&changelog, "CHANGELOG.md")
            .expect_err("missing unreleased heading should fail");
        assert!(
            error.contains("must contain a `## [Unreleased]` section"),
            "unexpected changelog error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn parse_cli_preserves_non_utf8_paths() {
        let repo_root_arg = OsString::from_vec(vec![0x72, 0x80, 0x70, 0x6f]);
        let commit_msg_arg = OsString::from_vec(vec![0x63, 0x80, 0x6d, 0x6d, 0x69, 0x74]);

        let command = parse_cli(
            [
                OsString::from("commit-msg"),
                OsString::from("--repo-root"),
                repo_root_arg.clone(),
                OsString::from("--commit-msg-file"),
                commit_msg_arg.clone(),
            ]
            .into_iter(),
        )
        .expect("parse cli");

        let CliCommand::CommitMsg {
            repo_root,
            commit_msg_file,
        } = command
        else {
            panic!("expected commit-msg command");
        };
        assert_eq!(repo_root, PathBuf::from(repo_root_arg));
        assert_eq!(commit_msg_file, PathBuf::from(commit_msg_arg));
    }

    fn run_ok(repo_root: &Path, args: &[&str], label: &str) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("{label}: failed to execute git: {error}"));
        assert!(
            output.status.success(),
            "{label}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
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
            let path = env::temp_dir().join(format!("repo-check-{prefix}-{nanos}-{unique}"));
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
}
