use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use toml::Value as TomlValue;

const TEMPLATE_VERSION: &str = "0.1.0";
const IGNORED_TEMPLATE_DIR_NAMES: &[&str] = &[".git", "target", "node_modules", "__pycache__"];
const PYTHON_RESERVED_KEYWORDS: &[&str] = &[
    "false", "none", "true", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

#[derive(Clone, Copy, Debug)]
struct EmbeddedTemplateFile {
    path: &'static str,
    contents: &'static [u8],
}

#[derive(Clone, Debug)]
enum TemplateSource {
    Filesystem(PathBuf),
    Embedded(&'static EmbeddedTemplateFile),
}

#[derive(Clone, Debug)]
struct TemplateFile {
    source: TemplateSource,
    relative_output: String,
}

type RenderedTemplateFiles = std::collections::BTreeMap<String, (usize, TemplateSource)>;
type TrackedTemplateFile = (usize, PathBuf, PathBuf);

include!(concat!(env!("OUT_DIR"), "/embedded_templates.rs"));

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NameKind {
    Repo,
    RustPackage,
    CrateDir,
    PythonImportPackage,
    DistributionPackage,
}

impl NameKind {
    fn label(self) -> &'static str {
        match self {
            Self::Repo => "repo name",
            Self::RustPackage => "Rust package name",
            Self::CrateDir => "crate directory",
            Self::PythonImportPackage => "Python import package name",
            Self::DistributionPackage => "distribution package name",
        }
    }

    fn delimiter(self) -> char {
        match self {
            Self::PythonImportPackage => '_',
            Self::Repo | Self::RustPackage | Self::CrateDir | Self::DistributionPackage => '-',
        }
    }
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

#[derive(Debug)]
struct StoredScaffoldConfig {
    template_version: String,
    config: InitConfig,
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

    fn primary_validation_command(&self) -> String {
        "cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local".to_string()
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
            preflight_init_environment(&config)?;
            prepare_target_dir(&config)?;
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
    let values: Vec<OsString> = args.collect();
    if values.is_empty() {
        return Err(usage());
    }

    let subcommand = utf8_arg(&values[0], "subcommand")?;
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
        match current.to_str() {
            Some("--project") => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                project_kind = ProjectKind::parse(utf8_arg(value, "--project value")?)?;
            }
            Some("--layout") => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                layout = Some(Layout::parse(utf8_arg(value, "--layout value")?)?);
            }
            Some("--repo-name") => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                repo_name = Some(utf8_arg(value, "--repo-name value")?.to_string());
            }
            Some("--package-name") => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                package_name = Some(utf8_arg(value, "--package-name value")?.to_string());
            }
            Some("--crate-dir") => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                crate_dir = Some(utf8_arg(value, "--crate-dir value")?.to_string());
            }
            Some("--force") => force = true,
            Some("--no-git-init") => git_init = false,
            Some("--no-setup-hooks") => setup_hooks = false,
            Some(value) if value.starts_with("--") => {
                return Err(format!("unsupported option: {value}"));
            }
            _ => {
                if target_dir.is_some() {
                    return Err(format!(
                        "unexpected positional argument: {}",
                        PathBuf::from(current).display()
                    ));
                }
                target_dir = Some(PathBuf::from(current));
            }
        }
        index += 1;
    }

    let target_dir = target_dir.ok_or_else(usage)?;
    let target_basename = target_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("new-project");
    let inferred_repo_name = match repo_name {
        Some(value) => normalize_name(&value, NameKind::Repo, true)?,
        None => normalize_name(target_basename, NameKind::Repo, false)
            .unwrap_or_else(|_| "new-project".to_string()),
    };
    let package_name = match package_name {
        Some(value) => normalize_package_name(&value, project_kind, true)?,
        None => derive_default_package_name(project_kind, &inferred_repo_name)?,
    };
    let crate_dir = match crate_dir {
        Some(value) => normalize_name(&value, NameKind::CrateDir, true)?,
        None => package_name.clone(),
    };
    let python_package = normalize_name(&package_name, NameKind::PythonImportPackage, false)?;

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

fn utf8_arg<'a>(value: &'a OsString, label: &str) -> Result<&'a str, String> {
    value.to_str().ok_or_else(|| {
        format!(
            "{label} must be valid UTF-8: {}",
            PathBuf::from(value).display()
        )
    })
}

fn derive_default_package_name(
    project_kind: ProjectKind,
    repo_name: &str,
) -> Result<String, String> {
    normalize_package_name(repo_name, project_kind, false)
}

fn normalize_package_name(
    value: &str,
    project_kind: ProjectKind,
    explicit: bool,
) -> Result<String, String> {
    match project_kind {
        ProjectKind::Rust => normalize_name(value, NameKind::RustPackage, explicit),
        ProjectKind::Python | ProjectKind::Nodejs => {
            normalize_name(value, NameKind::DistributionPackage, explicit)
        }
    }
}

fn normalize_name(value: &str, kind: NameKind, explicit: bool) -> Result<String, String> {
    let normalized = normalize_ascii_name(value, kind.delimiter())?;
    validate_normalized_name(value, &normalized, kind, explicit)?;
    Ok(normalized)
}

fn normalize_ascii_name(value: &str, delimiter: char) -> Result<String, String> {
    let lowered = value.trim().to_lowercase().replace(['_', ' '], "-");
    let mut out = String::new();
    let mut last_delimiter = false;
    for character in lowered.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            out.push(character);
            last_delimiter = false;
            continue;
        }
        if !last_delimiter {
            out.push(delimiter);
            last_delimiter = true;
        }
    }
    let result = out.trim_matches(delimiter).to_string();
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

fn embedded_template_prefixes(config: &InitConfig) -> Vec<String> {
    let mut paths = vec!["templates/common/".to_string()];
    match config.project_kind {
        ProjectKind::Rust => {
            paths.push(format!(
                "templates/projects/rust/{}/",
                config.layout.as_str()
            ));
        }
        ProjectKind::Python => {
            paths.push("templates/projects/python/root/".to_string());
        }
        ProjectKind::Nodejs => {
            paths.push("templates/projects/nodejs/root/".to_string());
        }
    }
    paths
}

fn output_manifest(config: &InitConfig) -> Result<Vec<String>, String> {
    let mut manifest: Vec<String> = template_files(config)?
        .into_iter()
        .map(|file| file.relative_output)
        .collect();
    manifest.sort();
    Ok(manifest)
}

fn template_files(config: &InitConfig) -> Result<Vec<TemplateFile>, String> {
    let template_roots = template_roots(config);
    template_files_with_roots(config, &template_roots)
}

fn template_files_with_roots(
    config: &InitConfig,
    template_roots: &[PathBuf],
) -> Result<Vec<TemplateFile>, String> {
    if template_roots.iter().all(|root| root.is_dir()) {
        return template_files_from_filesystem(config, template_roots);
    }
    embedded_template_files(config)
}

fn template_files_from_filesystem(
    config: &InitConfig,
    template_roots: &[PathBuf],
) -> Result<Vec<TemplateFile>, String> {
    let mut files = RenderedTemplateFiles::new();
    if let Some(tracked_files) = tracked_template_files(template_roots)? {
        for (root_index, template_root, source_path) in tracked_files {
            let relative_source = source_path.strip_prefix(&template_root).map_err(|error| {
                format!("failed to relativize {}: {error}", source_path.display())
            })?;
            let relative_output = render_path(relative_source, config)?;
            register_template_file(
                &mut files,
                root_index,
                TemplateSource::Filesystem(source_path),
                relative_output,
            )?;
        }
        return Ok(rendered_template_files(files));
    }

    for (root_index, root) in template_roots.iter().enumerate() {
        if !root.is_dir() {
            return Err(format!("missing template directory: {}", root.display()));
        }
        collect_template_files(config, root, root, root_index, &mut files)?;
    }
    Ok(rendered_template_files(files))
}

fn embedded_template_files(config: &InitConfig) -> Result<Vec<TemplateFile>, String> {
    let mut files = RenderedTemplateFiles::new();
    for (root_index, prefix) in embedded_template_prefixes(config).into_iter().enumerate() {
        let mut matched = false;
        for template in EMBEDDED_TEMPLATES
            .iter()
            .filter(|template| template.path.starts_with(&prefix))
        {
            matched = true;
            let relative_source = &template.path[prefix.len()..];
            let relative_output = render_string(relative_source, config);
            register_template_file(
                &mut files,
                root_index,
                TemplateSource::Embedded(template),
                relative_output,
            )?;
        }
        if !matched {
            return Err(format!("missing embedded template directory: {prefix}"));
        }
    }
    Ok(rendered_template_files(files))
}

fn rendered_template_files(files: RenderedTemplateFiles) -> Vec<TemplateFile> {
    files
        .into_iter()
        .map(|(relative_output, (_, source))| TemplateFile {
            source,
            relative_output,
        })
        .collect()
}

fn tracked_template_files(
    template_roots: &[PathBuf],
) -> Result<Option<Vec<TrackedTemplateFile>>, String> {
    let repo_root = repo_root();
    if !repo_root.join(".git").exists() {
        return Ok(None);
    }

    let mut files = Vec::new();
    for (root_index, template_root) in template_roots.iter().enumerate() {
        let repo_relative = template_root.strip_prefix(&repo_root).map_err(|error| {
            format!("failed to relativize {}: {error}", template_root.display())
        })?;
        let Some(tracked_paths) = git_ls_files(&repo_root, repo_relative)? else {
            return Ok(None);
        };
        for tracked_path in tracked_paths {
            let source_path = repo_root.join(&tracked_path);
            if !source_path.starts_with(template_root) {
                continue;
            }
            files.push((root_index, template_root.clone(), source_path));
        }
    }
    Ok(Some(files))
}

fn git_ls_files(repo_root: &Path, pathspec: &Path) -> Result<Option<Vec<PathBuf>>, String> {
    let normalized_pathspec = normalized_relative_template_path(pathspec)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .arg("-z")
        .arg("--")
        .arg(&normalized_pathspec)
        .output();

    let output = match output {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8(output.stdout).ok();
    let Some(stdout) = stdout else {
        return Ok(None);
    };

    Ok(Some(
        split_null_terminated_text(&stdout)
            .into_iter()
            .map(PathBuf::from)
            .collect(),
    ))
}

fn split_null_terminated_text(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect()
}

fn collect_template_files(
    config: &InitConfig,
    template_root: &Path,
    current: &Path,
    root_index: usize,
    files: &mut RenderedTemplateFiles,
) -> Result<(), String> {
    let mut entries = read_dir_entry_paths(current)?;
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            if should_skip_template_dir(&entry) {
                continue;
            }
            collect_template_files(config, template_root, &entry, root_index, files)?;
            continue;
        }
        let relative_source = entry
            .strip_prefix(template_root)
            .map_err(|error| format!("failed to relativize {}: {error}", entry.display()))?;
        let relative_output = render_path(relative_source, config)?;
        register_template_file(
            files,
            root_index,
            TemplateSource::Filesystem(entry),
            relative_output,
        )?;
    }
    Ok(())
}

fn register_template_file(
    files: &mut RenderedTemplateFiles,
    root_index: usize,
    source: TemplateSource,
    relative_output: String,
) -> Result<(), String> {
    if let Some((existing_root_index, _)) = files.get(&relative_output)
        && *existing_root_index == root_index
    {
        return Err(format!(
            "duplicate rendered template path detected: {relative_output}"
        ));
    }
    files.insert(relative_output, (root_index, source));
    Ok(())
}

fn should_skip_template_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| IGNORED_TEMPLATE_DIR_NAMES.contains(&name))
}

fn render_path(path: &Path, config: &InitConfig) -> Result<String, String> {
    Ok(render_string(
        &normalized_relative_template_path(path)?,
        config,
    ))
}

fn normalized_relative_template_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                parts.push(part.to_str().ok_or_else(|| {
                    format!("template path must be valid UTF-8: {}", path.display())
                })?)
            }
            _ => {
                return Err(format!(
                    "template path must stay within the template root: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(parts.join("/"))
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
        .replace(
            "__PRIMARY_VALIDATION_COMMAND__",
            &config.primary_validation_command(),
        )
}

fn prepare_target_dir(config: &InitConfig) -> Result<(), String> {
    fs::create_dir_all(&config.target_dir)
        .map_err(|error| format!("failed to create {}: {error}", config.target_dir.display()))?;
    let existing = existing_target_entries(&config.target_dir)?;
    if existing.is_empty() {
        return Ok(());
    }

    if config.force {
        return cleanup_existing_scaffold(config);
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

fn preflight_init_environment(config: &InitConfig) -> Result<(), String> {
    if !needs_git_command(config) {
        return Ok(());
    }
    ensure_command_available(
        "git",
        &["--version"],
        "git is required before writing files because the requested init/hooks flow would otherwise leave a partial scaffold behind",
    )
}

fn needs_git_command(config: &InitConfig) -> bool {
    config.setup_hooks || (config.git_init && !config.target_dir.join(".git").exists())
}

fn existing_target_entries(target_dir: &Path) -> Result<Vec<String>, String> {
    let mut existing: Vec<String> = read_dir_entries(target_dir)?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.file_name() != Some(OsStr::new(".git")))
        .map(|path| display_entry_name(&path))
        .collect();
    existing.sort();
    Ok(existing)
}

fn display_entry_name(path: &Path) -> String {
    path.file_name()
        .map(|name| Path::new(name).display().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn cleanup_existing_scaffold(config: &InitConfig) -> Result<(), String> {
    let Some(existing) = load_stored_scaffold_config(&config.target_dir)? else {
        return Err(
            "target directory is not empty, and `--force` only supports repositories previously generated by omne-project-init.\n\nExpected to find a valid repo-check.toml in the target directory."
                .to_string(),
        );
    };
    if existing.template_version != TEMPLATE_VERSION {
        return Err(format!(
            "target directory was generated by template version {}.\n\n`--force` only supports in-place regeneration from template version {} so stale files can be cleaned safely.",
            existing.template_version, TEMPLATE_VERSION
        ));
    }

    let old_manifest = output_manifest(&existing.config)?;
    let new_manifest = output_manifest(config)?;
    let modified_paths = modified_managed_paths(&config.target_dir, &existing.config)?;
    if !modified_paths.is_empty() {
        return Err(format!(
            "`--force` refused because previously generated files were modified by hand.\n\nRestore or remove these paths before re-running:\n{}",
            modified_paths
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let colliding_paths = unmanaged_new_path_collisions(&config.target_dir, &old_manifest, config)?;
    if !colliding_paths.is_empty() {
        return Err(format!(
            "`--force` refused because regeneration would overwrite non-generated files.\n\nConflicting paths:\n{}",
            colliding_paths
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let mut managed_paths = std::collections::BTreeSet::new();
    managed_paths.extend(old_manifest);
    managed_paths.extend(new_manifest);

    for relative_path in &managed_paths {
        remove_managed_path(&config.target_dir.join(relative_path))?;
    }
    prune_empty_generated_directories(&config.target_dir, &managed_paths)?;
    Ok(())
}

fn modified_managed_paths(target_dir: &Path, config: &InitConfig) -> Result<Vec<String>, String> {
    let mut modified = Vec::new();
    for template in template_files(config)? {
        let relative_output = template.relative_output;
        let destination = target_dir.join(&relative_output);
        if !destination.exists() {
            continue;
        }
        let expected = render_template_bytes(template_source_bytes(&template.source)?, config);
        let actual = fs::read(&destination)
            .map_err(|error| format!("failed to read {}: {error}", destination.display()))?;
        if actual != expected {
            modified.push(relative_output);
        }
    }
    modified.sort();
    Ok(modified)
}

fn unmanaged_new_path_collisions(
    target_dir: &Path,
    old_manifest: &[String],
    new_config: &InitConfig,
) -> Result<Vec<String>, String> {
    let old_paths: std::collections::BTreeSet<&str> =
        old_manifest.iter().map(String::as_str).collect();
    let mut collisions = Vec::new();
    for relative_output in output_manifest(new_config)? {
        if old_paths.contains(relative_output.as_str()) {
            continue;
        }
        if target_dir.join(&relative_output).exists() {
            collisions.push(relative_output);
        }
    }
    collisions.sort();
    Ok(collisions)
}

fn remove_managed_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("failed to remove directory {}: {error}", path.display()))?;
        return Ok(());
    }

    fs::remove_file(path)
        .map_err(|error| format!("failed to remove file {}: {error}", path.display()))
}

fn prune_empty_generated_directories(
    target_dir: &Path,
    managed_paths: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let mut directories = std::collections::BTreeSet::new();
    for relative_path in managed_paths {
        let mut current = Path::new(relative_path).parent();
        while let Some(parent) = current {
            if parent.as_os_str().is_empty() {
                break;
            }
            directories.insert(target_dir.join(parent));
            current = parent.parent();
        }
    }

    let mut directories: Vec<PathBuf> = directories.into_iter().collect();
    directories.sort_by(|left, right| {
        let left_depth = left.components().count();
        let right_depth = right.components().count();
        right_depth.cmp(&left_depth).then_with(|| right.cmp(left))
    });

    for directory in directories {
        if !directory.exists() {
            continue;
        }
        let is_empty = directory_is_empty(&directory)?;
        if is_empty {
            fs::remove_dir(&directory)
                .map_err(|error| format!("failed to remove {}: {error}", directory.display()))?;
        }
    }
    Ok(())
}

fn load_stored_scaffold_config(target_dir: &Path) -> Result<Option<StoredScaffoldConfig>, String> {
    let path = target_dir.join("repo-check.toml");
    if !path.is_file() {
        return Ok(None);
    }

    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let values = toml::from_str::<TomlValue>(&text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let template_version = required_value(&values, "template_version")?;
    let repo_name = required_value(&values, "repo_name")?;
    let project_kind = ProjectKind::parse(&required_value(&values, "project_kind")?)?;
    let layout = Layout::parse(&required_value(&values, "layout")?)?;
    let package_name = required_value(&values, "package_name")?;
    let crate_dir = required_value(&values, "crate_dir")?;
    let python_package = required_value(&values, "python_package")?;

    Ok(Some(StoredScaffoldConfig {
        template_version,
        config: InitConfig {
            target_dir: target_dir.to_path_buf(),
            repo_name,
            package_name,
            crate_dir,
            python_package,
            project_kind,
            layout,
            force: true,
            git_init: true,
            setup_hooks: true,
        },
    }))
}

fn required_value(values: &TomlValue, key: &str) -> Result<String, String> {
    values
        .get(key)
        .and_then(TomlValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing `{key}` in repo-check.toml"))
}

fn read_dir_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?
    {
        let entry =
            entry.map_err(|error| format!("failed to read {} entry: {error}", path.display()))?;
        entries.push(entry);
    }
    Ok(entries)
}

fn read_dir_entry_paths(path: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(read_dir_entries(path)?
        .into_iter()
        .map(|entry| entry.path())
        .collect())
}

fn directory_is_empty(path: &Path) -> Result<bool, String> {
    Ok(read_dir_entries(path)?.is_empty())
}

fn write_files(config: &InitConfig) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for template in template_files(config)? {
        let destination = config.target_dir.join(&template.relative_output);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let content = template_source_bytes(&template.source)?;
        let rendered = render_template_bytes(content, config);
        fs::write(&destination, rendered)
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        maybe_mark_executable(&destination, &template.relative_output)?;
        written.push(template.relative_output);
    }
    Ok(written)
}

fn template_source_bytes(source: &TemplateSource) -> Result<Vec<u8>, String> {
    match source {
        TemplateSource::Filesystem(path) => {
            fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
        }
        TemplateSource::Embedded(template) => Ok(template.contents.to_vec()),
    }
}

fn render_template_bytes(content: Vec<u8>, config: &InitConfig) -> Vec<u8> {
    match String::from_utf8(content) {
        Ok(text) => render_string(&text, config).into_bytes(),
        Err(error) => error.into_bytes(),
    }
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

fn ensure_command_available(program: &str, args: &[&str], reason: &str) -> Result<(), String> {
    match Command::new(program).args(args).output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            if detail.is_empty() {
                Err(format!(
                    "failed to probe `{program}` before initialization: {reason}"
                ))
            } else {
                Err(format!(
                    "failed to probe `{program}` before initialization: {reason}\n\n{detail}"
                ))
            }
        }
        Err(error) => Err(format!(
            "failed to execute `{program}` before initialization: {reason}\n\n{error}"
        )),
    }
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

fn validate_normalized_name(
    original: &str,
    normalized: &str,
    kind: NameKind,
    explicit: bool,
) -> Result<(), String> {
    if matches!(kind, NameKind::RustPackage | NameKind::PythonImportPackage)
        && normalized
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
    {
        let flag = match kind {
            NameKind::RustPackage
            | NameKind::DistributionPackage
            | NameKind::PythonImportPackage => "--package-name",
            NameKind::CrateDir => "--crate-dir",
            NameKind::Repo => "--repo-name",
        };
        let source = if explicit { "provided" } else { "derived" };
        return Err(format!(
            "{source} {} is invalid: `{}` normalizes to `{}` which starts with a digit.\n\nChoose a value whose first ASCII character is a letter, for example via `{flag}`.",
            kind.label(),
            original,
            normalized
        ));
    }
    if kind == NameKind::PythonImportPackage && PYTHON_RESERVED_KEYWORDS.contains(&normalized) {
        let source = if explicit { "provided" } else { "derived" };
        return Err(format!(
            "{source} {} is invalid: `{}` normalizes to `{}` which is a reserved Python keyword.\n\nChoose a different `{}` via `--package-name`.",
            kind.label(),
            original,
            normalized,
            kind.label()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn collect_template_files_skips_known_build_directories() {
        let sandbox = TempDir::new("template-scan");
        let template_root = sandbox.path().join("templates");
        fs::create_dir_all(template_root.join("target").join("debug")).expect("create target dir");
        fs::create_dir_all(template_root.join("src")).expect("create src dir");
        fs::write(template_root.join("src").join("main.rs"), "fn main() {}\n")
            .expect("write source template");
        fs::write(
            template_root.join("target").join("debug").join("binary"),
            [0_u8, 159, 146, 150],
        )
        .expect("write ignored binary artifact");

        let config = test_config(sandbox.path());
        let mut files = std::collections::BTreeMap::new();
        collect_template_files(&config, &template_root, &template_root, 0, &mut files)
            .expect("collect template files");

        let rendered_paths: Vec<String> = files.into_keys().collect();
        assert_eq!(rendered_paths, vec!["src/main.rs"]);
    }

    #[test]
    fn later_template_roots_override_common_templates() {
        let sandbox = TempDir::new("template-override");
        let common_root = sandbox.path().join("templates").join("common");
        let project_root = sandbox.path().join("templates").join("project");
        fs::create_dir_all(&common_root).expect("create common root");
        fs::create_dir_all(&project_root).expect("create project root");
        fs::write(common_root.join("README.md"), "common\n").expect("write common README");
        fs::write(project_root.join("README.md"), "project\n").expect("write project README");

        let config = test_config(sandbox.path());
        let mut files = std::collections::BTreeMap::new();
        collect_template_files(&config, &common_root, &common_root, 0, &mut files)
            .expect("collect common templates");
        collect_template_files(&config, &project_root, &project_root, 1, &mut files)
            .expect("collect project templates");

        let Some((root_index, source_path)) = files.get("README.md") else {
            panic!("expected merged README.md template");
        };
        assert_eq!(*root_index, 1);
        match source_path {
            TemplateSource::Filesystem(path) => assert_eq!(path, &project_root.join("README.md")),
            TemplateSource::Embedded(_) => panic!("expected filesystem template source"),
        }
    }

    #[test]
    fn rust_package_names_reject_leading_digits() {
        let error = normalize_name("123-demo", NameKind::RustPackage, true)
            .expect_err("leading-digit rust package should fail");
        assert!(error.contains("starts with a digit"));
        assert_eq!(
            normalize_name("123-demo", NameKind::CrateDir, true).expect("normalize crate dir"),
            "123-demo"
        );
    }

    #[test]
    fn python_import_names_reject_leading_digits() {
        let error = normalize_name("123-demo", NameKind::PythonImportPackage, true)
            .expect_err("leading-digit python import package should fail");
        assert!(error.contains("starts with a digit"));
    }

    #[test]
    fn python_import_names_reject_reserved_keywords() {
        let error = normalize_name("async", NameKind::PythonImportPackage, true)
            .expect_err("python keyword should fail");
        assert!(error.contains("reserved Python keyword"));
    }

    #[test]
    fn render_path_normalizes_template_paths_to_forward_slashes() {
        let config = test_config(Path::new("."));
        let rendered = render_path(Path::new("nested/path/__CRATE_DIR__/Cargo.toml"), &config)
            .expect("render template path");
        assert_eq!(rendered, "nested/path/demo-repo/Cargo.toml");
    }

    #[test]
    fn render_template_bytes_keeps_binary_assets_verbatim() {
        let config = test_config(Path::new("."));
        let rendered = render_template_bytes(vec![0xff, 0xfe, 0xfd], &config);
        assert_eq!(rendered, vec![0xff, 0xfe, 0xfd]);
    }

    #[test]
    fn embedded_template_catalog_covers_supported_projects() {
        let sandbox = TempDir::new("embedded-templates");

        let rust_manifest = embedded_template_files(&test_config(sandbox.path()))
            .expect("embedded rust templates")
            .into_iter()
            .map(|file| file.relative_output)
            .collect::<Vec<_>>();
        assert!(rust_manifest.iter().any(|path| path == "Cargo.toml"));
        assert!(
            rust_manifest
                .iter()
                .any(|path| path == "crates/demo-repo/Cargo.toml")
        );

        let python_manifest = embedded_template_files(&InitConfig {
            project_kind: ProjectKind::Python,
            layout: Layout::Root,
            ..test_config(sandbox.path())
        })
        .expect("embedded python templates")
        .into_iter()
        .map(|file| file.relative_output)
        .collect::<Vec<_>>();
        assert!(python_manifest.iter().any(|path| path == "pyproject.toml"));
        assert!(
            python_manifest
                .iter()
                .any(|path| path == "demo_repo/__init__.py")
        );

        let node_manifest = embedded_template_files(&InitConfig {
            project_kind: ProjectKind::Nodejs,
            layout: Layout::Root,
            ..test_config(sandbox.path())
        })
        .expect("embedded node templates")
        .into_iter()
        .map(|file| file.relative_output)
        .collect::<Vec<_>>();
        assert!(node_manifest.iter().any(|path| path == "package.json"));
        assert!(node_manifest.iter().any(|path| path == "src/index.js"));
    }

    #[test]
    fn template_files_fall_back_to_embedded_templates_when_disk_templates_are_missing() {
        let sandbox = TempDir::new("embedded-fallback");
        let missing_roots = vec![
            sandbox.path().join("templates").join("common"),
            sandbox
                .path()
                .join("templates")
                .join("projects")
                .join("rust")
                .join("crate"),
        ];

        let manifest = template_files_with_roots(&test_config(sandbox.path()), &missing_roots)
            .expect("fallback to embedded templates")
            .into_iter()
            .map(|file| file.relative_output)
            .collect::<Vec<_>>();
        assert!(manifest.iter().any(|path| path == "README.md"));
        assert!(
            manifest
                .iter()
                .any(|path| path == "crates/demo-repo/Cargo.toml")
        );
    }

    #[test]
    fn needs_git_command_only_when_git_work_is_requested() {
        let sandbox = TempDir::new("git-preflight");
        assert!(!needs_git_command(&test_config(sandbox.path())));

        let git_init = InitConfig {
            git_init: true,
            ..test_config(sandbox.path())
        };
        assert!(needs_git_command(&git_init));

        fs::create_dir_all(sandbox.path().join(".git")).expect("create .git");
        assert!(!needs_git_command(&git_init));

        let setup_hooks = InitConfig {
            setup_hooks: true,
            ..test_config(sandbox.path())
        };
        assert!(needs_git_command(&setup_hooks));
    }

    #[test]
    fn template_files_ignore_untracked_repo_artifacts_when_git_metadata_is_available() {
        let _guard = repo_mutation_lock()
            .lock()
            .expect("lock repo mutation guard");
        let local_dir = repo_root()
            .join("templates")
            .join("common")
            .join("local-artifacts");
        let _cleanup = CleanupPath(local_dir.clone());
        fs::create_dir_all(&local_dir).expect("create local artifact dir");
        fs::write(
            local_dir.join("note.txt"),
            "do not treat this as a template\n",
        )
        .expect("write local artifact");

        let manifest = output_manifest(&test_config(Path::new("."))).expect("render manifest");
        assert!(
            !manifest
                .iter()
                .any(|path| path == "local-artifacts/note.txt"),
            "manifest unexpectedly included an untracked template artifact: {manifest:?}"
        );
    }

    #[test]
    fn derived_rust_package_names_reject_invalid_repo_slug() {
        let error = derive_default_package_name(ProjectKind::Rust, "123-demo")
            .expect_err("derived rust package should fail");
        assert!(error.contains("--package-name"));
    }

    #[test]
    fn load_stored_scaffold_config_accepts_real_toml_with_tables() {
        let sandbox = TempDir::new("stored-config-toml");
        fs::write(
            sandbox.path().join("repo-check.toml"),
            concat!(
                "template_version = \"0.1.0\"\n",
                "repo_name = \"demo-repo\"\n",
                "project_kind = \"rust\"\n",
                "layout = \"crate\"\n",
                "package_name = \"demo-repo\"\n",
                "crate_dir = \"demo-repo\"\n",
                "python_package = \"demo_repo\"\n",
                "package_manifest_path = \"crates/demo-repo/Cargo.toml\"\n",
                "changelog_path = \"crates/demo-repo/CHANGELOG.md\"\n",
                "\n",
                "[extra]\n",
                "owner = \"foundation\"\n",
            ),
        )
        .expect("write repo-check.toml");

        let stored = load_stored_scaffold_config(sandbox.path())
            .expect("load stored scaffold config")
            .expect("stored scaffold config should exist");
        assert_eq!(stored.template_version, TEMPLATE_VERSION);
        assert_eq!(stored.config.repo_name, "demo-repo");
        assert_eq!(stored.config.project_kind, ProjectKind::Rust);
        assert_eq!(stored.config.layout, Layout::Crate);
    }

    #[cfg(unix)]
    #[test]
    fn parse_cli_preserves_non_utf8_target_dir() {
        let target_dir = OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]);
        let command = parse_cli(
            [
                OsString::from("manifest"),
                target_dir.clone(),
                OsString::from("--project"),
                OsString::from("rust"),
            ]
            .into_iter(),
        )
        .expect("parse cli");

        let CliCommand::Manifest(config) = command else {
            panic!("expected manifest command");
        };
        assert_eq!(config.target_dir, PathBuf::from(target_dir));
    }

    #[cfg(unix)]
    #[test]
    fn prepare_target_dir_rejects_non_utf8_existing_entries() {
        let sandbox = TempDir::new("non-utf8-existing");
        fs::write(
            sandbox
                .path()
                .join(PathBuf::from(OsString::from_vec(vec![0x66, 0x80, 0x6f]))),
            "existing\n",
        )
        .expect("write existing file");

        let error = prepare_target_dir(&test_config(sandbox.path()))
            .expect_err("non-empty directory should be rejected");
        assert!(error.contains("target directory is not empty"));
    }

    fn test_config(target_dir: &Path) -> InitConfig {
        InitConfig {
            target_dir: target_dir.to_path_buf(),
            repo_name: "demo-repo".to_string(),
            package_name: "demo-repo".to_string(),
            crate_dir: "demo-repo".to_string(),
            python_package: "demo_repo".to_string(),
            project_kind: ProjectKind::Rust,
            layout: Layout::Crate,
            force: false,
            git_init: false,
            setup_hooks: false,
        }
    }

    fn repo_mutation_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct TempDir {
        path: PathBuf,
    }

    struct CleanupPath(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos();
            let path = env::temp_dir().join(format!("omne-project-init-{prefix}-{nanos}-{unique}"));
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

    impl Drop for CleanupPath {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
