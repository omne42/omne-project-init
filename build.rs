use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IGNORED_TEMPLATE_DIR_NAMES: &[&str] = &[".git", "target", "node_modules", "__pycache__"];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let templates_dir = manifest_dir.join("templates");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", templates_dir.display());

    let template_files = collect_template_files(&manifest_dir, &templates_dir)
        .expect("collect embedded template files");
    let output = render_embedded_templates(&manifest_dir, &template_files)
        .expect("render embedded template catalog");
    let out_path =
        PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("embedded_templates.rs");
    fs::write(&out_path, output).expect("write embedded template catalog");
}

fn collect_template_files(
    manifest_dir: &Path,
    templates_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    if let Some(tracked) = git_tracked_template_files(manifest_dir)? {
        return Ok(tracked);
    }

    let mut files = Vec::new();
    collect_template_files_from_disk(templates_dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn git_tracked_template_files(manifest_dir: &Path) -> Result<Option<Vec<PathBuf>>, String> {
    if !manifest_dir.join(".git").exists() {
        return Ok(None);
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .arg("ls-files")
        .arg("-z")
        .arg("--")
        .arg("templates")
        .output()
        .map_err(|error| format!("failed to run git ls-files for templates: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("git ls-files returned non-utf8 output: {error}"))?;
    let mut files: Vec<PathBuf> = stdout
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(|value| manifest_dir.join(value))
        .collect();
    files.sort();
    Ok(Some(files))
}

fn collect_template_files_from_disk(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(root)
        .map_err(|error| format!("failed to read {}: {error}", root.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("failed to read {} entry: {error}", root.display()))
        })
        .collect::<Result<_, _>>()?;
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            if should_skip_template_dir(&entry) {
                continue;
            }
            collect_template_files_from_disk(&entry, files)?;
            continue;
        }
        files.push(entry);
    }
    Ok(())
}

fn should_skip_template_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| IGNORED_TEMPLATE_DIR_NAMES.contains(&name))
}

fn render_embedded_templates(
    manifest_dir: &Path,
    template_files: &[PathBuf],
) -> Result<String, String> {
    let mut output = String::from("static EMBEDDED_TEMPLATES: &[EmbeddedTemplateFile] = &[\n");
    for path in template_files {
        let repo_relative = path
            .strip_prefix(manifest_dir)
            .map_err(|error| format!("failed to relativize {}: {error}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        let absolute = path
            .canonicalize()
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))?
            .to_string_lossy()
            .to_string();
        output.push_str(&format!(
            "    EmbeddedTemplateFile {{ path: {:?}, contents: include_bytes!({:?}) }},\n",
            repo_relative, absolute
        ));
    }
    output.push_str("];\n");
    Ok(output)
}
