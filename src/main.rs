use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const TEMPLATE_VERSION: &str = "0.1.0";
const IGNORED_TEMPLATE_DIR_NAMES: &[&str] = &[".git", "target", "node_modules", "__pycache__"];

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
                repo_name = Some(value.to_string());
            }
            "--package-name" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                package_name = Some(value.to_string());
            }
            "--crate-dir" => {
                index += 1;
                let value = values.get(index).ok_or_else(usage)?;
                crate_dir = Some(value.to_string());
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
    let mut entries = read_dir_entry_paths(current)?;
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            if should_skip_template_dir(&entry) {
                continue;
            }
            collect_template_files(config, template_root, &entry, files, seen)?;
            continue;
        }
        let relative_source = entry
            .strip_prefix(template_root)
            .map_err(|error| format!("failed to relativize {}: {error}", entry.display()))?;
        let relative_output = render_path(relative_source, config)?;
        if !seen.insert(relative_output.clone()) {
            return Err(format!(
                "duplicate rendered template path detected: {relative_output}"
            ));
        }
        files.push((entry, relative_output));
    }
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

fn existing_target_entries(target_dir: &Path) -> Result<Vec<String>, String> {
    let mut existing: Vec<String> = read_dir_entries(target_dir)?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some(".git"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|value| value.to_string())
        })
        .collect();
    existing.sort();
    Ok(existing)
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
    for (source_path, relative_output) in template_files(config)? {
        let destination = target_dir.join(&relative_output);
        if !destination.exists() {
            continue;
        }
        let expected = render_template_bytes(
            fs::read(&source_path)
                .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?,
            config,
        );
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
    let old_paths: std::collections::BTreeSet<&str> = old_manifest.iter().map(String::as_str).collect();
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
    let values = parse_flat_config(&text)?;
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

fn parse_flat_config(text: &str) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut values = std::collections::BTreeMap::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!(
                "invalid config line {} in repo-check.toml: expected `key = \"value\"`",
                line_number + 1
            )
        })?;
        let key = key.trim();
        let value = parse_quoted_value(value.trim()).ok_or_else(|| {
            format!(
                "invalid config value on line {} in repo-check.toml: expected quoted string",
                line_number + 1
            )
        })?;
        values.insert(key.to_string(), value);
    }
    Ok(values)
}

fn required_value(
    values: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> Result<String, String> {
    values
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing `{key}` in repo-check.toml"))
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
    for (source_path, relative_output) in template_files(config)? {
        let destination = config.target_dir.join(&relative_output);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let content = fs::read(&source_path)
            .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
        let rendered = render_template_bytes(content, config);
        fs::write(&destination, rendered)
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        maybe_mark_executable(&destination, &relative_output)?;
        written.push(relative_output);
    }
    Ok(written)
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
            NameKind::RustPackage | NameKind::DistributionPackage | NameKind::PythonImportPackage => {
                "--package-name"
            }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut files = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        collect_template_files(
            &config,
            &template_root,
            &template_root,
            &mut files,
            &mut seen,
        )
        .expect("collect template files");

        let rendered_paths: Vec<String> = files.into_iter().map(|(_, relative)| relative).collect();
        assert_eq!(rendered_paths, vec!["src/main.rs"]);
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
    fn derived_rust_package_names_reject_invalid_repo_slug() {
        let error = derive_default_package_name(ProjectKind::Rust, "123-demo")
            .expect_err("derived rust package should fail");
        assert!(error.contains("--package-name"));
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
}
