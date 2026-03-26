use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TEMPLATE_VERSION: &str = "0.1.0";

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
            _ => Err(format!("unsupported project kind: {value}")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Nodejs => "nodejs",
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
            _ => Err(format!("unsupported layout: {value}")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Crate => "crate",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Root => "root-package repository",
            Self::Crate => "crate-package directory",
        }
    }
}

#[derive(Debug)]
enum CliCommand {
    Init(InitConfig),
    Manifest(InitConfig),
}

#[derive(Debug)]
struct InitConfig {
    target_dir: PathBuf,
    repo_name: String,
    package_name: String,
    crate_dir: String,
    python_package: String,
    project_kind: ProjectKind,
    layout: Layout,
    force: bool,
    git_init: bool,
    setup_hooks: bool,
}

impl InitConfig {
    fn changelog_path(&self) -> String {
        match self.layout {
            Layout::Root => "CHANGELOG.md".to_string(),
            Layout::Crate => format!("crates/{}/CHANGELOG.md", self.crate_dir),
        }
    }

    fn package_manifest_path(&self) -> String {
        match (self.project_kind, self.layout) {
            (ProjectKind::Rust, Layout::Root) => "Cargo.toml".to_string(),
            (ProjectKind::Rust, Layout::Crate) => format!("crates/{}/Cargo.toml", self.crate_dir),
            (ProjectKind::Python, _) => "pyproject.toml".to_string(),
            (ProjectKind::Nodejs, _) => "package.json".to_string(),
        }
    }

    fn primary_source_path(&self) -> String {
        match (self.project_kind, self.layout) {
            (ProjectKind::Rust, Layout::Root) => "src/main.rs".to_string(),
            (ProjectKind::Rust, Layout::Crate) => {
                format!("crates/{}/src/lib.rs", self.crate_dir)
            }
            (ProjectKind::Python, _) => format!("{}/__init__.py", self.python_package),
            (ProjectKind::Nodejs, _) => "src/index.js".to_string(),
        }
    }

    fn primary_test_path(&self) -> String {
        match (self.project_kind, self.layout) {
            (ProjectKind::Rust, Layout::Root) => "src/main.rs".to_string(),
            (ProjectKind::Rust, Layout::Crate) => {
                format!("crates/{}/src/lib.rs", self.crate_dir)
            }
            (ProjectKind::Python, _) => "tests/test_basic.py".to_string(),
            (ProjectKind::Nodejs, _) => "test/basic.test.js".to_string(),
        }
    }
}

fn main() {
    if let Err(error) = real_main() {
        eprintln!("omne-project-init: {error}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), String> {
    let command = parse_cli(env::args_os().skip(1))?;
    match command {
        CliCommand::Init(config) => {
            ensure_target_ready(&config)?;
            let written = write_files(&config)?;
            maybe_init_git_repo(&config)?;
            maybe_setup_hooks(&config)?;
            println!(
                "Initialized repo scaffold at: {}",
                config.target_dir.display()
            );
            println!("Template version: {TEMPLATE_VERSION}");
            println!("Project kind: {}", config.project_kind.as_str());
            println!("Layout: {}", config.layout.label());
            println!("Package manifest: {}", config.package_manifest_path());
            println!("Changelog path: {}", config.changelog_path());
            println!("Generated files: {}", written.len());
        }
        CliCommand::Manifest(config) => {
            let manifest = output_manifest(&config)?;
            print_json_array(&manifest);
        }
    }
    Ok(())
}

fn parse_cli(args: impl Iterator<Item = OsString>) -> Result<CliCommand, String> {
    let values: Vec<String> = args
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    if values.is_empty() {
        return Err(usage());
    }

    let subcommand = values[0].as_str();
    let mut project_kind = ProjectKind::Rust;
    let mut layout: Option<Layout> = None;
    let mut repo_name: Option<String> = None;
    let mut package_name: Option<String> = None;
    let mut crate_dir: Option<String> = None;
    let mut force = false;
    let mut git_init = true;
    let mut setup_hooks = true;
    let mut target_dir: Option<PathBuf> = None;

    let mut index = 1;
    while index < values.len() {
        let current = &values[index];
        match current.as_str() {
            "--project" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                project_kind = ProjectKind::parse(value)?;
            }
            "--layout" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                layout = Some(Layout::parse(value)?);
            }
            "--repo-name" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                repo_name = Some(slugify(value)?);
            }
            "--package-name" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                package_name = Some(slugify(value)?);
            }
            "--crate-dir" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                crate_dir = Some(slugify(value)?);
            }
            "--force" => force = true,
            "--no-git-init" => git_init = false,
            "--no-setup-hooks" => setup_hooks = false,
            value if value.starts_with("--") => return Err(format!("unsupported option: {value}")),
            value => {
                if target_dir.is_some() {
                    return Err(format!("unexpected positional argument: {value}"));
                }
                target_dir = Some(PathBuf::from(value));
            }
        }
        index += 1;
    }

    let target_dir = target_dir.ok_or_else(usage)?;
    let inferred_repo_name = repo_name.unwrap_or_else(|| {
        slugify(
            target_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("new-project"),
        )
        .unwrap_or_else(|_| "new-project".to_string())
    });
    let package_name = package_name.unwrap_or_else(|| inferred_repo_name.clone());
    let crate_dir = crate_dir.unwrap_or_else(|| package_name.clone());
    let python_package = package_name.replace('-', "_");

    let layout = match layout {
        Some(Layout::Crate) if project_kind != ProjectKind::Rust => {
            return Err("crate layout is only supported for rust projects".to_string());
        }
        Some(value) => value,
        None => match project_kind {
            ProjectKind::Rust => Layout::Crate,
            ProjectKind::Python | ProjectKind::Nodejs => Layout::Root,
        },
    };

    let config = InitConfig {
        target_dir: target_dir
            .canonicalize()
            .unwrap_or_else(|_| target_dir.clone()),
        repo_name: inferred_repo_name,
        package_name,
        crate_dir,
        python_package,
        project_kind,
        layout,
        force,
        git_init,
        setup_hooks,
    };

    match subcommand {
        "init" => Ok(CliCommand::Init(config)),
        "manifest" => Ok(CliCommand::Manifest(config)),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage: omne-project-init <init|manifest> <target_dir> [--project rust|python|nodejs] [--layout root|crate] [--repo-name NAME] [--package-name NAME] [--crate-dir NAME] [--force] [--no-git-init] [--no-setup-hooks]".to_string()
}

fn slugify(value: &str) -> Result<String, String> {
    let lowered = value
        .trim()
        .to_lowercase()
        .replace('_', "-")
        .replace(' ', "-");
    let mut out = String::new();
    let mut last_dash = false;
    for character in lowered.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            out.push(character);
            last_dash = false;
            continue;
        }
        if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let result = out.trim_matches('-').to_string();
    if result.is_empty() {
        return Err("name must contain ASCII letters or digits".to_string());
    }
    Ok(result)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn template_roots(config: &InitConfig) -> Vec<PathBuf> {
    let root = repo_root();
    let mut paths = vec![root.join("templates").join("common")];
    match config.project_kind {
        ProjectKind::Rust => {
            paths.push(
                root.join("templates")
                    .join("projects")
                    .join("rust")
                    .join(config.layout.as_str()),
            );
        }
        ProjectKind::Python => {
            paths.push(
                root.join("templates")
                    .join("projects")
                    .join("python")
                    .join("root"),
            );
        }
        ProjectKind::Nodejs => {
            paths.push(
                root.join("templates")
                    .join("projects")
                    .join("nodejs")
                    .join("root"),
            );
        }
    }
    paths
}

fn output_manifest(config: &InitConfig) -> Result<Vec<String>, String> {
    let mut manifest: Vec<String> = template_files(config)?
        .into_iter()
        .map(|(_, relative)| relative)
        .collect();
    manifest.sort();
    Ok(manifest)
}

fn template_files(config: &InitConfig) -> Result<Vec<(PathBuf, String)>, String> {
    let mut files = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for root in template_roots(config) {
        if !root.is_dir() {
            return Err(format!("missing template directory: {}", root.display()));
        }
        collect_template_files(config, &root, &root, &mut files, &mut seen)?;
    }
    Ok(files)
}

fn collect_template_files(
    config: &InitConfig,
    template_root: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, String)>,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(current)
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .collect();
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            collect_template_files(config, template_root, &entry, files, seen)?;
            continue;
        }
        let relative_source = entry
            .strip_prefix(template_root)
            .map_err(|error| format!("failed to relativize {}: {error}", entry.display()))?;
        let relative_output = render_path(relative_source, config);
        if !seen.insert(relative_output.clone()) {
            return Err(format!(
                "duplicate rendered template path detected: {relative_output}"
            ));
        }
        files.push((entry, relative_output));
    }
    Ok(())
}

fn render_path(path: &Path, config: &InitConfig) -> String {
    let text = path.to_string_lossy();
    render_string(&text, config)
}

fn render_string(value: &str, config: &InitConfig) -> String {
    value
        .replace("__TEMPLATE_VERSION__", TEMPLATE_VERSION)
        .replace("__REPO_NAME__", &config.repo_name)
        .replace("__PACKAGE_NAME__", &config.package_name)
        .replace("__CRATE_DIR__", &config.crate_dir)
        .replace("__PY_PACKAGE__", &config.python_package)
        .replace("__PROJECT_KIND__", config.project_kind.as_str())
        .replace("__LAYOUT__", config.layout.as_str())
        .replace("__LAYOUT_LABEL__", config.layout.label())
        .replace("__CHANGELOG_PATH__", &config.changelog_path())
        .replace("__PACKAGE_MANIFEST_PATH__", &config.package_manifest_path())
        .replace("__PRIMARY_SOURCE_PATH__", &config.primary_source_path())
        .replace("__PRIMARY_TEST_PATH__", &config.primary_test_path())
}

fn ensure_target_ready(config: &InitConfig) -> Result<(), String> {
    fs::create_dir_all(&config.target_dir)
        .map_err(|error| format!("failed to create {}: {error}", config.target_dir.display()))?;
    if config.force {
        return Ok(());
    }

    let mut existing: Vec<String> = fs::read_dir(&config.target_dir)
        .map_err(|error| format!("failed to read {}: {error}", config.target_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some(".git"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|value| value.to_string())
        })
        .collect();
    existing.sort();

    if existing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "target directory is not empty. Re-run with --force to overwrite generated files.\n\nExisting entries:\n{}",
        existing
            .into_iter()
            .map(|entry| format!("- {entry}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn write_files(config: &InitConfig) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (source_path, relative_output) in template_files(config)? {
        let destination = config.target_dir.join(&relative_output);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let content = fs::read_to_string(&source_path)
            .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
        fs::write(&destination, render_string(&content, config))
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        maybe_mark_executable(&destination, &relative_output)?;
        written.push(relative_output);
    }
    Ok(written)
}

fn maybe_mark_executable(path: &Path, relative_output: &str) -> Result<(), String> {
    if !matches!(
        relative_output,
        "githooks/pre-commit" | "githooks/commit-msg"
    ) {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| format!("failed to read metadata {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("failed to set permissions {}: {error}", path.display()))?;
    }

    Ok(())
}

fn maybe_init_git_repo(config: &InitConfig) -> Result<(), String> {
    if !config.git_init || config.target_dir.join(".git").exists() {
        return Ok(());
    }
    let init_with_branch = run_command(
        Command::new("git")
            .arg("init")
            .arg("-b")
            .arg("main")
            .current_dir(&config.target_dir),
    );
    if init_with_branch.is_ok() {
        return Ok(());
    }

    run_command(
        Command::new("git")
            .arg("init")
            .current_dir(&config.target_dir),
    )?;
    let _ = run_command(
        Command::new("git")
            .arg("branch")
            .arg("-m")
            .arg("main")
            .current_dir(&config.target_dir),
    );
    Ok(())
}

fn maybe_setup_hooks(config: &InitConfig) -> Result<(), String> {
    if !config.setup_hooks || !config.target_dir.join(".git").exists() {
        return Ok(());
    }
    run_command(
        Command::new("git")
            .arg("-C")
            .arg(&config.target_dir)
            .arg("config")
            .arg("core.hooksPath")
            .arg("githooks"),
    )
}

fn run_command(command: &mut Command) -> Result<(), String> {
    let rendered = format!("{command:?}");
    let output = command
        .output()
        .map_err(|error| format!("failed to execute {rendered}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.trim().is_empty() {
        return Err(format!("command failed: {rendered}"));
    }
    Err(stderr.trim().to_string())
}

fn print_json_array(values: &[String]) {
    println!("[");
    for (index, value) in values.iter().enumerate() {
        let suffix = if index + 1 == values.len() { "" } else { "," };
        println!("  \"{}\"{suffix}", json_escape(value));
    }
    println!("]");
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}
