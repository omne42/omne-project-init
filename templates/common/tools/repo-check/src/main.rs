use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

#[derive(Debug)]
struct RepoConfig {
    template_version: String,
    repo_name: String,
    project_kind: ProjectKind,
    layout: Layout,
    package_name: String,
    crate_dir: String,
    python_package: String,
    package_manifest_path: String,
    changelog_path: String,
}

#[derive(Debug)]
struct StagedState {
    paths: Vec<String>,
    deleted_paths: BTreeSet<String>,
    changelog_paths: Vec<String>,
    non_changelog_count: usize,
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
            run_workspace_checks(&repo_root, &config, mode)
        }
        CliCommand::ValidateBranch { repo_root } => {
            validate_branch_name(&normalize_repo_root(repo_root))
        }
    }
}

fn parse_cli(args: impl Iterator<Item = OsString>) -> Result<CliCommand, String> {
    let values: Vec<String> = args
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    if values.is_empty() {
        return Err(usage());
    }

    let subcommand = values[0].as_str();
    let mut repo_root = PathBuf::from(".");
    let mut commit_msg_file: Option<PathBuf> = None;
    let mut workspace_mode: Option<WorkspaceMode> = None;

    let mut index = 1;
    if subcommand == "workspace" {
        let value = values.get(index).ok_or_else(usage)?;
        workspace_mode = Some(WorkspaceMode::parse(value)?);
        index += 1;
    }

    while index < values.len() {
        match values[index].as_str() {
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

fn normalize_repo_root(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

impl RepoConfig {
    fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join("repo-check.toml");
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("repo-check: failed to read {}: {error}", path.display()))?;
        let values = parse_flat_config(&text)?;

        let template_version = required_value(&values, "template_version")?;
        let repo_name = required_value(&values, "repo_name")?;
        let project_kind = ProjectKind::parse(&required_value(&values, "project_kind")?)?;
        let layout = Layout::parse(&required_value(&values, "layout")?)?;
        let package_name = required_value(&values, "package_name")?;
        let crate_dir = required_value(&values, "crate_dir")?;
        let python_package = required_value(&values, "python_package")?;
        let package_manifest_path = required_value(&values, "package_manifest_path")?;
        let changelog_path = required_value(&values, "changelog_path")?;

        Ok(Self {
            template_version,
            repo_name,
            project_kind,
            layout,
            package_name,
            crate_dir,
            python_package,
            package_manifest_path,
            changelog_path,
        })
    }
}

fn required_value(values: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    values
        .get(key)
        .cloned()
        .ok_or_else(|| format!("repo-check: missing `{key}` in repo-check.toml"))
}

fn parse_flat_config(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!(
                "repo-check: invalid config line {}: expected `key = \"value\"`",
                line_number + 1
            )
        })?;
        let key = key.trim();
        let value = parse_quoted_value(value.trim()).ok_or_else(|| {
            format!(
                "repo-check: invalid config value on line {}: expected quoted string",
                line_number + 1
            )
        })?;
        values.insert(key.to_string(), value);
    }
    Ok(values)
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, character) in line.char_indices() {
        match character {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

fn parse_quoted_value(value: &str) -> Option<String> {
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return None;
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::new();
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            match character {
                '\\' | '"' => out.push(character),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                _ => return None,
            }
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        out.push(character);
    }
    if escaped {
        return None;
    }
    Some(out)
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
    let staged = collect_staged_state(repo_root)?;
    if staged.paths.is_empty() {
        return Ok(());
    }

    require_major_bump_override(repo_root, config)?;
    validate_allowed_changelog_paths(config, &staged)?;
    validate_required_changelog_not_deleted(config, &staged)?;
    validate_changelog_update(repo_root, config, &staged)?;
    validate_not_changelog_only(&staged)?;
    validate_released_sections_immutable(repo_root, config, &staged)?;

    run_workspace_checks(repo_root, config, WorkspaceMode::Local)
}

fn run_commit_msg(
    repo_root: &Path,
    config: &RepoConfig,
    commit_msg_file: &Path,
) -> Result<(), String> {
    validate_branch_name(repo_root)?;
    let first_line = read_first_line(commit_msg_file)?;
    if is_special_commit_message(&first_line) {
        return Ok(());
    }

    let parsed = parse_conventional_commit(&first_line)?;
    require_breaking_commit_marker(repo_root, config, &parsed)
}

fn validate_layout_shape(repo_root: &Path, config: &RepoConfig) -> Result<(), String> {
    if config.layout == Layout::Crate && config.project_kind != ProjectKind::Rust {
        return Err("repo-check: crate layout is only supported for rust projects".to_string());
    }

    match (config.project_kind, config.layout) {
        (ProjectKind::Rust, Layout::Root) => {
            let manifest = repo_root.join("Cargo.toml");
            if !manifest.is_file() {
                return Err(
                    "repo-check: rust root layout requires Cargo.toml at repo root".to_string(),
                );
            }
        }
        (ProjectKind::Rust, Layout::Crate) => {
            let crate_dirs = discover_crate_dirs(repo_root)?;
            if crate_dirs.is_empty() {
                return Err(
                    "repo-check: rust crate layout requires at least one crates/*/Cargo.toml"
                        .to_string(),
                );
            }
        }
        (ProjectKind::Python, Layout::Root) => {
            if !repo_root.join("pyproject.toml").is_file() {
                return Err("repo-check: python root layout requires pyproject.toml".to_string());
            }
        }
        (ProjectKind::Nodejs, Layout::Root) => {
            if !repo_root.join("package.json").is_file() {
                return Err("repo-check: nodejs root layout requires package.json".to_string());
            }
        }
        _ => {}
    }

    Ok(())
}

fn validate_branch_name(repo_root: &Path) -> Result<(), String> {
    let branch = git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"], true)?;
    let branch = branch.trim();
    if branch.is_empty() || branch == "HEAD" {
        return Ok(());
    }
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

fn collect_staged_state(repo_root: &Path) -> Result<StagedState, String> {
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

    let paths = split_null_terminated(&changed);
    let deleted_paths: BTreeSet<String> = split_null_terminated(&deleted).into_iter().collect();
    let changelog_paths: Vec<String> = paths
        .iter()
        .filter(|path| is_changelog_path(path))
        .cloned()
        .collect();
    let non_changelog_count = paths.iter().filter(|path| !is_changelog_path(path)).count();
    let crate_dirs_with_non_changelog_changes = paths
        .iter()
        .filter(|path| !is_changelog_path(path))
        .filter_map(|path| crate_dir_for_path(path))
        .collect();

    Ok(StagedState {
        paths,
        deleted_paths,
        changelog_paths,
        non_changelog_count,
        crate_dirs_with_non_changelog_changes,
    })
}

fn split_null_terminated(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn is_changelog_path(path: &str) -> bool {
    path == "CHANGELOG.md" || (path.starts_with("crates/") && path.ends_with("/CHANGELOG.md"))
}

fn crate_dir_for_path(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    let first = parts.next()?;
    let second = parts.next()?;
    if first == "crates" {
        return Some(second.to_string());
    }
    None
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
                .filter(|path| path.as_str() != "CHANGELOG.md")
                .collect();
            if disallowed.is_empty() {
                return Ok(());
            }
            Err(format!(
                "repo-check: this repository uses a single root changelog.\n\nOnly `CHANGELOG.md` is allowed.\n\nDisallowed staged changelog paths:\n{}",
                bullet_list(disallowed.iter().map(|path| path.as_str()))
            ))
        }
        Layout::Crate => {
            let disallowed_root: Vec<&String> = staged
                .changelog_paths
                .iter()
                .filter(|path| {
                    path.as_str() == "CHANGELOG.md" && !staged.deleted_paths.contains(path.as_str())
                })
                .collect();
            let invalid: Vec<&String> = staged
                .changelog_paths
                .iter()
                .filter(|path| path.as_str() != "CHANGELOG.md")
                .filter(|path| !is_crate_changelog_path(path))
                .collect();
            if disallowed_root.is_empty() && invalid.is_empty() {
                return Ok(());
            }
            let mut details: Vec<&str> = disallowed_root
                .iter()
                .chain(invalid.iter())
                .map(|value| value.as_str())
                .collect();
            details.sort();
            Err(format!(
                "repo-check: this repository keeps changelogs inside each crate.\n\nRoot CHANGELOG.md is not a valid changelog entry point here.\n\nDisallowed staged changelog paths:\n{}",
                bullet_list(details.into_iter())
            ))
        }
    }
}

fn is_crate_changelog_path(path: &str) -> bool {
    path.starts_with("crates/") && path.ends_with("/CHANGELOG.md")
}

fn validate_required_changelog_not_deleted(
    config: &RepoConfig,
    staged: &StagedState,
) -> Result<(), String> {
    let deleted: Vec<&str> = match config.layout {
        Layout::Root => staged
            .deleted_paths
            .iter()
            .filter(|path| path.as_str() == "CHANGELOG.md")
            .map(|path| path.as_str())
            .collect(),
        Layout::Crate => staged
            .deleted_paths
            .iter()
            .filter(|path| is_crate_changelog_path(path))
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
            if staged
                .changelog_paths
                .iter()
                .any(|path| path == "CHANGELOG.md")
            {
                return Ok(());
            }
            Err(
                "repo-check: a root-package repository must update CHANGELOG.md in the same commit."
                    .to_string(),
            )
        }
        Layout::Crate => {
            if staged.crate_dirs_with_non_changelog_changes.is_empty() {
                return Ok(());
            }

            let mut missing_files = Vec::new();
            let mut missing_updates = Vec::new();

            for crate_dir in &staged.crate_dirs_with_non_changelog_changes {
                let changelog_path = format!("crates/{crate_dir}/CHANGELOG.md");
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
                    "repo-check: every crate-package must maintain its own changelog.\n\nCreate the missing changelog file(s):\n{}",
                    bullet_list(missing_files.iter().map(|path| path.as_str()))
                ));
            }
            if !missing_updates.is_empty() {
                return Err(format!(
                    "repo-check: every changed crate-package must update its own changelog.\n\nStage an [Unreleased] entry in:\n{}",
                    bullet_list(missing_updates.iter().map(|path| path.as_str()))
                ));
            }

            Ok(())
        }
    }
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
            .filter(|path| path.as_str() == "CHANGELOG.md")
            .collect(),
        Layout::Crate => staged
            .changelog_paths
            .iter()
            .filter(|path| is_crate_changelog_path(path))
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

        return Err(
            "repo-check: refusing to modify released CHANGELOG sections.\n\nOnly edit entries under [Unreleased].\nIf you are intentionally cutting a release, re-run with:\n  OMNE_ALLOW_CHANGELOG_RELEASE_EDIT=1 git commit ..."
                .to_string(),
        );
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
        "repo-check: refusing major version change by default.\n\nThe following targets changed their major segment:\n{}\n\nRe-run with:\n  OMNE_ALLOW_MAJOR_VERSION_BUMP=1 git commit ...",
        bullet_list(changed_targets.iter().map(format_version_target))
    ))
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
        "repo-check: major version change requires an explicit breaking commit message.\n\nTargets:\n{}\n\nUse Conventional Commits with `!`, for example:\n  refactor(core)!: start 1.0 transition",
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
        let old_parts = parse_semver(target.old_version.as_deref())?;
        let new_parts = parse_semver(target.new_version.as_deref())?;

        let old_major = old_parts.map(|parts| parts.0);
        let new_major = new_parts.map(|parts| parts.0);

        if old_major == Some(0) || new_major == Some(0) {
            continue;
        }
        if old_major.is_none() && new_major.is_some_and(|value| value > 0) {
            changed.push(target);
            continue;
        }
        if let (Some(old_major), Some(new_major)) = (old_major, new_major)
            && new_major > old_major
        {
            changed.push(target);
        }
    }
    Ok(changed)
}

fn version_targets(repo_root: &Path, config: &RepoConfig) -> Result<Vec<VersionTarget>, String> {
    match (config.project_kind, config.layout) {
        (ProjectKind::Rust, Layout::Root) => {
            let head_text = git_show_text(repo_root, "HEAD:Cargo.toml")?;
            let index_text = git_show_text(repo_root, ":Cargo.toml")?;
            Ok(vec![VersionTarget {
                label: config.package_name.clone(),
                path: "Cargo.toml".to_string(),
                old_version: cargo_package_version(head_text.as_deref(), None).0,
                new_version: cargo_package_version(index_text.as_deref(), None).0,
            }])
        }
        (ProjectKind::Rust, Layout::Crate) => {
            let head_root = git_show_text(repo_root, "HEAD:Cargo.toml")?;
            let index_root = git_show_text(repo_root, ":Cargo.toml")?;
            let head_workspace_version = cargo_workspace_version(head_root.as_deref());
            let index_workspace_version = cargo_workspace_version(index_root.as_deref());

            let mut targets = Vec::new();
            for crate_dir in discover_crate_dirs(repo_root)? {
                let path = format!("crates/{crate_dir}/Cargo.toml");
                let head_text = git_show_text(repo_root, &format!("HEAD:{path}"))?;
                let index_text = git_show_text(repo_root, &format!(":{path}"))?;
                if head_text.is_none() && index_text.is_none() {
                    continue;
                }
                targets.push(VersionTarget {
                    label: crate_dir.clone(),
                    path,
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
            let head_text = git_show_text(repo_root, "HEAD:pyproject.toml")?;
            let index_text = git_show_text(repo_root, ":pyproject.toml")?;
            Ok(vec![VersionTarget {
                label: config.package_name.clone(),
                path: "pyproject.toml".to_string(),
                old_version: pyproject_version(head_text.as_deref()),
                new_version: pyproject_version(index_text.as_deref()),
            }])
        }
        (ProjectKind::Nodejs, Layout::Root) => {
            let head_text = git_show_text(repo_root, "HEAD:package.json")?;
            let index_text = git_show_text(repo_root, ":package.json")?;
            Ok(vec![VersionTarget {
                label: config.package_name.clone(),
                path: "package.json".to_string(),
                old_version: package_json_version(head_text.as_deref()),
                new_version: package_json_version(index_text.as_deref()),
            }])
        }
        _ => Ok(Vec::new()),
    }
}

fn parse_semver(version: Option<&str>) -> Result<Option<(u64, u64, u64)>, String> {
    let Some(version) = version else {
        return Ok(None);
    };
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return Err(format!(
            "repo-check: unsupported non-semver version: {version}"
        ));
    }
    let major = parts[0]
        .parse::<u64>()
        .map_err(|_| format!("repo-check: invalid semver major segment: {version}"))?;
    let minor = parts[1]
        .parse::<u64>()
        .map_err(|_| format!("repo-check: invalid semver minor segment: {version}"))?;
    let patch = parts[2]
        .parse::<u64>()
        .map_err(|_| format!("repo-check: invalid semver patch segment: {version}"))?;
    Ok(Some((major, minor, patch)))
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
    cargo_section_value(text, "workspace.package", "version")
}

fn cargo_package_version(
    text: Option<&str>,
    workspace_version: Option<&str>,
) -> (Option<String>, bool) {
    let Some(text) = text else {
        return (None, false);
    };
    let mut current_section = "";
    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = &line[1..line.len() - 1];
            continue;
        }
        if current_section != "package" {
            continue;
        }
        if let Some(version) = cargo_assignment_value(line, "version") {
            return (Some(version), false);
        }
        let compact: String = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if compact == "version.workspace=true" {
            return (workspace_version.map(|value| value.to_string()), true);
        }
    }
    (None, false)
}

fn cargo_section_value(text: Option<&str>, section: &str, key: &str) -> Option<String> {
    let text = text?;
    let mut current_section = "";
    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = &line[1..line.len() - 1];
            continue;
        }
        if current_section == section
            && let Some(value) = cargo_assignment_value(line, key)
        {
            return Some(value);
        }
    }
    None
}

fn cargo_assignment_value(line: &str, key: &str) -> Option<String> {
    let (left, right) = line.split_once('=')?;
    if left.trim() != key {
        return None;
    }
    parse_quoted_value(right.trim())
}

fn pyproject_version(text: Option<&str>) -> Option<String> {
    cargo_section_value(text, "project", "version")
}

fn package_json_version(text: Option<&str>) -> Option<String> {
    let text = text?;
    for raw_line in text.lines() {
        let line = raw_line.trim().trim_end_matches(',');
        if !line.starts_with("\"version\"") {
            continue;
        }
        let (_, value) = line.split_once(':')?;
        return parse_quoted_value(value.trim());
    }
    None
}

fn discover_crate_dirs(repo_root: &Path) -> Result<Vec<String>, String> {
    let crates_dir = repo_root.join("crates");
    if !crates_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut crate_dirs = Vec::new();
    let entries = fs::read_dir(&crates_dir).map_err(|error| {
        format!(
            "repo-check: failed to read {}: {error}",
            crates_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "repo-check: failed to read {} entry: {error}",
                crates_dir.display()
            )
        })?;
        let path = entry.path();
        if !path.is_dir() || !path.join("Cargo.toml").is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            crate_dirs.push(name.to_string());
        }
    }
    crate_dirs.sort();
    Ok(crate_dirs)
}

fn read_first_line(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("repo-check: failed to read {}: {error}", path.display()))?;
    Ok(text
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
        .to_string())
}

fn is_special_commit_message(line: &str) -> bool {
    line.starts_with("Merge ")
        || line.starts_with("Revert \"")
        || line.starts_with("fixup! ")
        || line.starts_with("squash! ")
}

fn parse_conventional_commit(line: &str) -> Result<ParsedCommitMessage, String> {
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

    Ok(ParsedCommitMessage { breaking })
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
    config: &RepoConfig,
    mode: WorkspaceMode,
) -> Result<(), String> {
    eprintln!(
        "repo-check: running {:?} checks for {} ({:?}, template {}, manifest {}, changelog {})",
        mode,
        config.repo_name,
        config.project_kind,
        config.template_version,
        config.package_manifest_path,
        config.changelog_path
    );
    if config.layout == Layout::Crate {
        eprintln!("repo-check: primary crate dir {}", config.crate_dir);
    }

    match config.project_kind {
        ProjectKind::Rust => {
            run_named_command(
                repo_root,
                "rust fmt",
                "cargo",
                &["fmt", "--all", "--", "--check"],
            )?;
            run_named_command(
                repo_root,
                "rust check",
                "cargo",
                &["check", "--workspace", "--all-targets", "--all-features"],
            )?;
            run_named_command(
                repo_root,
                "rust test",
                "cargo",
                &["test", "--workspace", "--all-features"],
            )?;
            if mode == WorkspaceMode::Ci {
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
                )?;
            }
            Ok(())
        }
        ProjectKind::Python => {
            let python = detect_python_command(repo_root)?;
            run_prefixed_command(
                repo_root,
                "python compileall",
                &python,
                &["-m", "compileall", &config.python_package, "tests"],
            )?;
            run_prefixed_command(
                repo_root,
                "python unittest",
                &python,
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
                &["--check", "src/index.js"],
            )?;
            run_named_command(repo_root, "node test", "node", &["--test"])
        }
    }
}

fn detect_python_command(repo_root: &Path) -> Result<Vec<String>, String> {
    let candidates = [
        vec!["python".to_string()],
        vec!["python3".to_string()],
        vec!["py".to_string(), "-3".to_string()],
    ];

    for candidate in candidates {
        if probe_prefixed_command(repo_root, &candidate, &["--version"]) {
            return Ok(candidate);
        }
    }

    Err("repo-check: unable to locate a Python interpreter. Tried `python`, `python3`, and `py -3`.".to_string())
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

fn probe_prefixed_command(repo_root: &Path, prefix: &[String], probe_args: &[&str]) -> bool {
    let Some(program) = prefix.first() else {
        return false;
    };
    let mut command = Command::new(program);
    command.current_dir(repo_root);
    if prefix.len() > 1 {
        command.args(&prefix[1..]);
    }
    command.args(probe_args);
    matches!(command.output(), Ok(output) if output.status.success())
}

fn run_named_command(
    repo_root: &Path,
    label: &str,
    program: &str,
    args: &[&str],
) -> Result<(), String> {
    eprintln!("repo-check: {label}");
    let mut command = Command::new(program);
    command.current_dir(repo_root).args(args);
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

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "repo-check: git command failed: git -C {} {}\n\n{}",
        repo_root.display(),
        args.join(" "),
        detail
    ))
}

fn git_show_text(repo_root: &Path, spec: &str) -> Result<Option<String>, String> {
    let output = run_git(repo_root, &["show", spec])?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(args);
    command
        .output()
        .map_err(|error| format!("repo-check: failed to execute git {:?}: {error}", args))
}

fn bullet_list(values: impl Iterator<Item = impl AsRef<str>>) -> String {
    values
        .map(|value| format!("- {}", value.as_ref()))
        .collect::<Vec<_>>()
        .join("\n")
}
