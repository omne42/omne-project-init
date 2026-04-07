use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn manifest_lists_expected_files_for_supported_projects() {
    let sandbox = TempDir::new("manifest-sandbox");

    let rust_output = run_cli([
        "manifest",
        sandbox.path().join("rust-crate").to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
    ]);
    assert_manifest_matches_templates(
        &rust_output,
        ManifestSpec {
            project_kind: ProjectKind::Rust,
            layout: Layout::Crate,
            repo_name: "rust-crate",
            package_name: "rust-crate",
            crate_dir: "rust-crate",
            python_package: "rust_crate",
        },
    );

    let python_output = run_cli([
        "manifest",
        sandbox.path().join("python-app").to_string_lossy().as_ref(),
        "--project",
        "python",
    ]);
    assert_manifest_matches_templates(
        &python_output,
        ManifestSpec {
            project_kind: ProjectKind::Python,
            layout: Layout::Root,
            repo_name: "python-app",
            package_name: "python-app",
            crate_dir: "python-app",
            python_package: "python_app",
        },
    );

    let node_output = run_cli([
        "manifest",
        sandbox.path().join("node-app").to_string_lossy().as_ref(),
        "--project",
        "nodejs",
    ]);
    assert_manifest_matches_templates(
        &node_output,
        ManifestSpec {
            project_kind: ProjectKind::Nodejs,
            layout: Layout::Root,
            repo_name: "node-app",
            package_name: "node-app",
            crate_dir: "node-app",
            python_package: "node_app",
        },
    );
}

#[test]
fn init_writes_expected_metadata_for_rust_layouts() {
    let rust_crate = init_repo(
        "rust-crate-layout",
        &["--project", "rust", "--layout", "crate"],
    );
    let rust_root = init_repo(
        "rust-root-layout",
        &["--project", "rust", "--layout", "root"],
    );
    let rust_crate_slug = repo_slug(rust_crate.path());

    let crate_repo_check = fs::read_to_string(rust_crate.path().join("repo-check.toml"))
        .expect("failed to read crate repo-check.toml");
    assert!(crate_repo_check.contains("schema_version = \"1\""));
    assert!(crate_repo_check.contains("layout = \"crate\""));
    assert!(crate_repo_check.contains(&format!(
        "package_manifest_path = \"crates/{rust_crate_slug}/Cargo.toml\""
    )));
    assert!(crate_repo_check.contains(&format!(
        "changelog_path = \"crates/{rust_crate_slug}/CHANGELOG.md\""
    )));

    let crate_workspace = fs::read_to_string(rust_crate.path().join("Cargo.toml"))
        .expect("failed to read crate workspace Cargo.toml");
    assert!(crate_workspace.contains(&format!("members = [\"crates/{rust_crate_slug}\"]")));
    assert!(crate_workspace.contains("exclude = [\"tools/repo-check\"]"));
    assert!(crate_workspace.contains("resolver = \"3\""));
    assert!(rust_crate.path().join("Cargo.lock").is_file());

    let crate_manifest = fs::read_to_string(
        rust_crate
            .path()
            .join("crates")
            .join(rust_crate_slug)
            .join("Cargo.toml"),
    )
    .expect("failed to read crate package Cargo.toml");
    assert!(crate_manifest.contains("edition = \"2024\""));
    assert!(crate_manifest.contains("version.workspace = true"));
    assert!(
        rust_crate
            .path()
            .join("crates")
            .join(rust_crate_slug)
            .join("tests/basic.rs")
            .is_file()
    );

    let root_repo_check = fs::read_to_string(rust_root.path().join("repo-check.toml"))
        .expect("failed to read root repo-check.toml");
    assert!(root_repo_check.contains("schema_version = \"1\""));
    assert!(root_repo_check.contains("layout = \"root\""));
    assert!(root_repo_check.contains("package_manifest_path = \"Cargo.toml\""));
    assert!(root_repo_check.contains("changelog_path = \"CHANGELOG.md\""));

    let root_manifest = fs::read_to_string(rust_root.path().join("Cargo.toml"))
        .expect("failed to read root Cargo.toml");
    assert!(root_manifest.contains("edition = \"2024\""));
    assert!(root_manifest.contains("exclude = [\"tools/repo-check\"]"));
    assert!(rust_root.path().join("Cargo.lock").is_file());
    assert!(rust_root.path().join("tests/basic.rs").is_file());

    let repo_check_manifest =
        fs::read_to_string(rust_root.path().join("tools/repo-check/Cargo.toml"))
            .expect("failed to read generated repo-check Cargo.toml");
    assert!(repo_check_manifest.contains("edition = \"2024\""));
    assert!(repo_check_manifest.contains("[workspace]"));
    assert!(
        rust_root
            .path()
            .join("tools/repo-check/Cargo.lock")
            .is_file()
    );
    assert!(
        rust_crate
            .path()
            .join("tools/repo-check/Cargo.lock")
            .is_file()
    );

    let crate_agents = fs::read_to_string(rust_crate.path().join("AGENTS.md"))
        .expect("failed to read crate AGENTS.md");
    assert!(crate_agents.contains(
        "主要验证命令：`cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local`"
    ));

    let root_agents =
        fs::read_to_string(rust_root.path().join("AGENTS.md")).expect("failed to read AGENTS.md");
    assert!(root_agents.contains(
        "主要验证命令：`cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local`"
    ));
}

#[test]
fn generated_rust_repo_check_workspace_local_passes_for_root_and_crate() {
    let rust_crate = init_repo(
        "rust-crate-smoke",
        &["--project", "rust", "--layout", "crate"],
    );
    let rust_root = init_repo(
        "rust-root-smoke",
        &["--project", "rust", "--layout", "root"],
    );

    run_generated_repo_check(rust_crate.path(), &["workspace", "local"]);
    run_generated_repo_check(rust_root.path(), &["workspace", "local"]);
}

#[test]
fn generated_rust_workspaces_exclude_repo_check_member() {
    let rust_crate = init_repo(
        "rust-crate-workspace-member",
        &["--project", "rust", "--layout", "crate"],
    );
    let rust_root = init_repo(
        "rust-root-workspace-member",
        &["--project", "rust", "--layout", "root"],
    );

    let crate_metadata = cargo_metadata(rust_crate.path());
    assert!(
        !crate_metadata.contains("/tools/repo-check/Cargo.toml"),
        "crate layout metadata unexpectedly included tools/repo-check:\n{crate_metadata}"
    );

    let root_metadata = cargo_metadata(rust_root.path());
    assert!(
        !root_metadata.contains("/tools/repo-check/Cargo.toml"),
        "root layout metadata unexpectedly included tools/repo-check:\n{root_metadata}"
    );
}

#[test]
fn generated_rust_repo_check_workspace_ci_passes_for_root_and_crate() {
    let rust_crate = init_repo(
        "rust-crate-ci-smoke",
        &["--project", "rust", "--layout", "crate"],
    );
    let rust_root = init_repo(
        "rust-root-ci-smoke",
        &["--project", "rust", "--layout", "root"],
    );

    run_generated_repo_check(rust_crate.path(), &["workspace", "ci"]);
    run_generated_repo_check(rust_root.path(), &["workspace", "ci"]);
}

#[test]
fn generated_python_repo_check_workspace_local_passes_when_supported_python_is_available() {
    if !has_supported_python() {
        eprintln!("skipping python smoke test: no Python 3.11+ interpreter found");
        return;
    }

    let repo = init_repo("python-smoke", &["--project", "python"]);
    run_generated_repo_check(repo.path(), &["workspace", "local"]);
}

#[test]
fn generated_python_repo_check_rejects_requires_python_below_template_floor() {
    let repo = init_repo("python-requires-floor", &["--project", "python"]);
    replace_in_file(
        &repo.path().join("pyproject.toml"),
        "requires-python = \">=3.11\"",
        "requires-python = \">=3.10\"",
    );

    let output = run_generated_repo_check_failure(repo.path(), &["workspace", "local"]);
    assert!(
        output.contains("project.requires-python` compatible with >=3.11"),
        "expected requires-python floor failure, got:\n{output}"
    );
}

#[test]
fn generated_node_repo_check_workspace_local_passes_when_node_is_available() {
    if !command_works("node", &["--version"]) {
        eprintln!("skipping node smoke test: `node` not found");
        return;
    }

    let repo = init_repo("node-smoke", &["--project", "nodejs"]);
    run_generated_repo_check(repo.path(), &["workspace", "local"]);
}

#[test]
fn generated_node_repo_check_uses_configured_primary_source_path() {
    if !command_works("node", &["--version"]) {
        eprintln!("skipping node primary source path test: `node` not found");
        return;
    }

    let repo = init_repo("node-primary-source", &["--project", "nodejs"]);
    fs::rename(
        repo.path().join("src/index.js"),
        repo.path().join("src/app.js"),
    )
    .expect("failed to rename node entrypoint");
    replace_in_file(
        &repo.path().join("test/basic.test.js"),
        "../src/index.js",
        "../src/app.js",
    );
    replace_in_file(
        &repo.path().join("repo-check.toml"),
        "primary_source_path = \"src/index.js\"",
        "primary_source_path = \"src/app.js\"",
    );

    run_generated_repo_check(repo.path(), &["workspace", "local"]);
}

#[test]
fn generated_rust_repo_check_git_flow_passes() {
    if !command_works("git", &["--version"]) {
        eprintln!("skipping git flow smoke test: `git` not found");
        return;
    }

    let repo = init_repo("rust-git-flow", &["--project", "rust", "--layout", "crate"]);
    git_init(repo.path());
    run_ok(
        "git checkout feature branch",
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("checkout")
            .arg("-b")
            .arg("feat/smoke-check"),
    );
    run_generated_repo_check(repo.path(), &["install-hooks"]);

    let hooks_path = run_ok(
        "git config core.hooksPath",
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("config")
            .arg("--get")
            .arg("core.hooksPath"),
    );
    assert_eq!(hooks_path.trim(), "githooks");

    run_ok(
        "git add",
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("add")
            .arg("."),
    );

    run_generated_repo_check(repo.path(), &["pre-commit"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.test");
    fs::write(&commit_msg, "feat(repo): initial scaffold\n")
        .expect("failed to write commit message file");
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
fn rust_package_names_stabilize_numeric_defaults_and_flags() {
    let sandbox = TempDir::new("numeric-rust-names");

    let derived_target = sandbox.path().join("123-derived-rust");
    run_cli([
        "init",
        derived_target.to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
    ]);
    assert!(
        derived_target
            .join("crates/app-123-derived-rust/Cargo.toml")
            .is_file()
    );

    let explicit_target = sandbox.path().join("rust-explicit");
    run_cli([
        "init",
        explicit_target.to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--package-name",
        "123-explicit-rust",
        "--crate-dir",
        "123-explicit-dir",
        "--no-git-init",
    ]);
    let explicit_manifest = explicit_target.join("crates/123-explicit-dir/Cargo.toml");
    assert!(explicit_manifest.is_file());
    assert!(
        fs::read_to_string(explicit_manifest)
            .expect("read explicit rust manifest")
            .contains("name = \"app-123-explicit-rust\"")
    );
}

#[test]
fn python_import_package_name_stabilizes_numeric_distribution_defaults() {
    let sandbox = TempDir::new("numeric-python-names");
    let derived_target = sandbox.path().join("123-python-derived");
    run_cli([
        "init",
        derived_target.to_string_lossy().as_ref(),
        "--project",
        "python",
        "--no-git-init",
    ]);
    assert!(
        derived_target
            .join("pkg_123_python_derived/__init__.py")
            .is_file()
    );

    let target = sandbox.path().join("python-explicit");
    run_cli([
        "init",
        target.to_string_lossy().as_ref(),
        "--project",
        "python",
        "--package-name",
        "123-python-app",
        "--no-git-init",
    ]);
    assert!(target.join("pkg_123_python_app/__init__.py").is_file());
}

#[test]
fn python_import_package_name_stabilizes_reserved_keywords() {
    let sandbox = TempDir::new("python-keyword-names");

    let derived_target = sandbox.path().join("async");
    run_cli([
        "init",
        derived_target.to_string_lossy().as_ref(),
        "--project",
        "python",
        "--no-git-init",
    ]);
    assert!(derived_target.join("pkg_async/__init__.py").is_file());

    let explicit_target = sandbox.path().join("python-explicit-keyword");
    run_cli([
        "init",
        explicit_target.to_string_lossy().as_ref(),
        "--project",
        "python",
        "--package-name",
        "async",
        "--no-git-init",
    ]);
    assert!(explicit_target.join("pkg_async/__init__.py").is_file());
}

#[test]
fn init_ignores_template_build_artifacts_with_non_utf8_bytes() {
    let _guard = template_fixture_lock()
        .lock()
        .expect("template fixture lock poisoned");
    let fixture_root = repo_template_root()
        .join("common")
        .join("tools")
        .join("repo-check")
        .join("target")
        .join("test-fixture");
    let _cleanup = PathCleanup::new(fixture_root.clone());
    fs::create_dir_all(&fixture_root).expect("failed to create template fixture directory");
    fs::write(fixture_root.join("bad.bin"), [0xff, 0xfe, 0xfd])
        .expect("failed to write non-UTF8 fixture");

    let repo = init_repo("artifact-skip", &["--project", "rust", "--layout", "crate"]);
    assert!(
        repo.path().join("tools/repo-check/src/main.rs").is_file(),
        "generated repo-check source was not written"
    );
    assert!(
        !repo.path().join("tools/repo-check/target").exists(),
        "template build artifacts leaked into generated scaffold"
    );
}

#[test]
fn force_reinit_replaces_previous_scaffold_instead_of_mixing_layouts() {
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
    assert!(repo.path().join("tests/test_basic.py").is_file());
    assert!(!repo.path().join("Cargo.toml").exists());
    assert!(!repo.path().join("crates").exists());
}

#[test]
fn force_reinit_refuses_to_remove_modified_generated_files() {
    let repo = TempDir::new("force-reinit-dirty");
    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
    ]);

    append_to_file(&repo.path().join("README.md"), "\nmanual note\n");
    let output = run_cli_failure([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
    ]);
    assert!(output.contains("previously generated files were modified by hand"));
    assert!(output.contains("README.md"));
}

#[test]
fn force_reinit_reports_modified_repo_check_toml_when_config_uses_tables() {
    let repo = TempDir::new("force-reinit-config-toml");
    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
    ]);

    append_to_file(
        &repo.path().join("repo-check.toml"),
        "\n[extra]\nowner = \"foundation\"\n",
    );

    let output = run_cli_failure([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
    ]);
    assert!(output.contains("previously generated files were modified by hand"));
    assert!(output.contains("repo-check.toml"));
    assert!(!output.contains("invalid config line"));
}

#[test]
fn force_reinit_refuses_non_generated_directories() {
    let repo = TempDir::new("force-reinit-non-generated");
    fs::write(repo.path().join("README.md"), "manual scratch dir\n")
        .expect("failed to seed non-generated directory");

    let output = run_cli_failure([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
    ]);
    assert!(
        output.contains("only supports repositories previously generated by omne-project-init")
    );
    assert!(output.contains("repo-check.toml"));
}

#[test]
fn force_reinit_refuses_to_overwrite_non_generated_colliding_paths() {
    let repo = TempDir::new("force-reinit-collision");
    run_cli([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
        "--no-git-init",
    ]);

    fs::write(
        repo.path().join("pyproject.toml"),
        "[project]\nname = \"manual\"\n",
    )
    .expect("failed to seed colliding non-generated file");

    let output = run_cli_failure([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "python",
        "--force",
        "--no-git-init",
    ]);
    assert!(output.contains("regeneration would overwrite non-generated files"));
    assert!(output.contains("pyproject.toml"));
}

#[test]
fn init_refuses_missing_git_before_writing_any_files() {
    let repo = TempDir::new("missing-git-preflight");
    let mut command = Command::new(cli_binary());
    command.args([
        "init",
        repo.path().to_string_lossy().as_ref(),
        "--project",
        "rust",
        "--layout",
        "crate",
    ]);
    command.env("PATH", "");
    command.env("Path", "");

    let output = run_fail("omne-project-init", &mut command);
    assert!(
        output.contains("before initialization"),
        "expected git preflight failure, got:\n{output}"
    );
    assert!(
        fs::read_dir(repo.path())
            .expect("read target dir after failed init")
            .next()
            .is_none(),
        "failed init left generated files behind"
    );
}

#[test]
fn generated_agents_use_validation_commands_instead_of_fake_test_paths() {
    let rust_root = init_repo(
        "rust-root-agents",
        &["--project", "rust", "--layout", "root"],
    );
    let python = init_repo("python-agents", &["--project", "python"]);

    let rust_agents =
        fs::read_to_string(rust_root.path().join("AGENTS.md")).expect("failed to read AGENTS");
    assert!(rust_agents.contains(
        "主要验证命令：`cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local`"
    ));
    assert!(!rust_agents.contains("主要验证入口"));

    let python_agents =
        fs::read_to_string(python.path().join("AGENTS.md")).expect("failed to read AGENTS");
    assert!(python_agents.contains(
        "主要验证命令：`cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local`"
    ));
}

#[test]
fn generated_docs_include_map_entrypoints_and_branch_protection_policy() {
    let repo = init_repo("docs-map", &["--project", "rust", "--layout", "root"]);

    let readme = fs::read_to_string(repo.path().join("README.md")).expect("failed to read README");
    let agents = fs::read_to_string(repo.path().join("AGENTS.md")).expect("failed to read AGENTS");
    let docs_readme =
        fs::read_to_string(repo.path().join("docs/README.md")).expect("failed to read docs README");
    let docs_map = fs::read_to_string(repo.path().join("docs/docs-system-map.md"))
        .expect("failed to read docs system map");
    let branch_rules = fs::read_to_string(repo.path().join("docs/规范/提交与分支.md"))
        .expect("failed to read branch policy doc");
    let hook_rules = fs::read_to_string(repo.path().join("docs/规范/Hook与质量门禁.md"))
        .expect("failed to read hook policy doc");

    assert!(readme.contains("docs/docs-system-map.md"));
    assert!(agents.contains("docs/docs-system-map.md"));
    assert!(docs_readme.contains("./docs-system-map.md"));
    assert!(docs_map.contains("docs/规范/提交与分支.md"));
    assert!(branch_rules.contains("必需的 CI / CD status checks 全部通过"));
    assert!(
        hook_rules
            .contains("本地 hook 和 `repo-check workspace local` 负责把高频问题尽量前移到开发机")
    );
}

#[test]
fn generated_repo_check_uses_configured_manifest_and_changelog_paths() {
    let repo = init_repo("node-config-paths", &["--project", "nodejs"]);
    git_init(repo.path());
    git_config_identity(repo.path());
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    replace_in_file(
        &repo.path().join("package.json"),
        "\"version\": \"0.1.0\"",
        "\"version\": \"1.2.3\"",
    );
    git_commit_all(repo.path(), "chore(node): prepare stable baseline");

    fs::create_dir_all(repo.path().join("config")).expect("failed to create config dir");
    fs::create_dir_all(repo.path().join("docs")).expect("failed to create docs dir");
    fs::rename(
        repo.path().join("package.json"),
        repo.path().join("config/package.json"),
    )
    .expect("failed to relocate package.json");
    fs::rename(
        repo.path().join("CHANGELOG.md"),
        repo.path().join("docs/CHANGELOG.md"),
    )
    .expect("failed to relocate changelog");
    replace_in_file(
        &repo.path().join("repo-check.toml"),
        "package_manifest_path = \"package.json\"",
        "package_manifest_path = \"config/package.json\"",
    );
    replace_in_file(
        &repo.path().join("repo-check.toml"),
        "changelog_path = \"CHANGELOG.md\"",
        "changelog_path = \"docs/CHANGELOG.md\"",
    );
    fs::write(
        repo.path().join("config/package.json"),
        "{\"name\":\"node-config-paths\",\"version\":\"2.0.0\",\"type\":\"module\"}\n",
    )
    .expect("failed to write package.json");
    append_to_file(
        &repo.path().join("docs/CHANGELOG.md"),
        "\n- note configured root manifest and changelog path move\n",
    );

    git_add_all(repo.path());
    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("config/package.json"),
        "configured manifest path was not used in version gate:\n{output}"
    );
    assert!(
        !output.contains("Only `CHANGELOG.md` is allowed"),
        "configured changelog path was ignored:\n{output}"
    );
    assert!(
        !output.contains("nodejs root layout requires package.json"),
        "configured manifest path was ignored:\n{output}"
    );
}

#[test]
fn generated_repo_check_requires_primary_changelog_for_root_changes_in_crate_layout() {
    let repo = init_repo(
        "crate-root-change",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    let primary_changelog = format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()));
    append_to_file(&repo.path().join("README.md"), "\nroot governance change\n");
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains(&primary_changelog),
        "expected configured primary changelog requirement, got:\n{output}"
    );

    append_to_file(
        &repo.path().join(&primary_changelog),
        "\n- note root governance change\n",
    );
    git_add_all(repo.path());
    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn generated_repo_check_uses_renamed_primary_changelog_for_root_changes_in_crate_layout() {
    let repo = init_repo(
        "crate-root-renamed",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    let primary_crate = repo_slug(repo.path());
    let old_changelog = format!("crates/{primary_crate}/CHANGELOG.md");
    let new_changelog = format!("crates/{primary_crate}/HISTORY.md");
    fs::rename(
        repo.path().join(&old_changelog),
        repo.path().join(&new_changelog),
    )
    .expect("failed to rename primary changelog");
    replace_in_file(
        &repo.path().join("repo-check.toml"),
        &format!("changelog_path = \"{old_changelog}\""),
        &format!("changelog_path = \"{new_changelog}\""),
    );
    git_add_all(repo.path());
    git_commit_all(repo.path(), "chore(repo): rename primary changelog");

    append_to_file(&repo.path().join("README.md"), "\nroot governance change\n");
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains(&new_changelog),
        "expected renamed primary changelog requirement, got:\n{output}"
    );
    assert!(
        !output.contains(&old_changelog),
        "stale default changelog path leaked into output:\n{output}"
    );

    append_to_file(
        &repo.path().join(&new_changelog),
        "\n- note root governance change after changelog rename\n",
    );
    git_add_all(repo.path());
    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn generated_repo_check_does_not_invent_fake_crates_from_shared_dirs() {
    let repo = init_repo(
        "crate-shared-dir",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    let primary_changelog = format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()));
    let shared_dir = repo.path().join("crates/shared");
    fs::create_dir_all(&shared_dir).expect("failed to create shared dir");
    fs::write(shared_dir.join("README.md"), "shared helper docs\n")
        .expect("failed to write shared doc");
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains(&primary_changelog),
        "expected root/shared changes to map to primary changelog, got:\n{output}"
    );
    assert!(
        !output.contains("crates/shared/CHANGELOG.md"),
        "shared directory was incorrectly treated as a crate:\n{output}"
    );
}

#[test]
fn generated_repo_check_ignores_non_member_scratch_crates() {
    let repo = init_repo(
        "crate-scratch-nonmember",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    let scratch_dir = repo.path().join("crates/scratch/src");
    fs::create_dir_all(&scratch_dir).expect("failed to create scratch src dir");
    fs::write(
        repo.path().join("crates/scratch/Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"scratch\"\n",
            "version = \"9.0.0\"\n",
            "edition = \"2024\"\n",
            "\n",
            "[lib]\n",
            "path = \"src/lib.rs\"\n",
        ),
    )
    .expect("failed to write scratch manifest");
    fs::write(scratch_dir.join("lib.rs"), "pub fn scratch() {}\n")
        .expect("failed to write scratch source");
    append_to_file(
        &repo
            .path()
            .join(format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()))),
        "\n- note scratch crate docs experiment\n",
    );
    git_add_all(repo.path());

    let output = run_generated_repo_check(repo.path(), &["pre-commit"]);
    assert!(
        !output.contains("crates/scratch/CHANGELOG.md"),
        "scratch crate outside workspace was treated as an active package:\n{output}"
    );
}

#[test]
fn generated_repo_check_requires_all_inheriting_crate_changelogs_for_workspace_version_change() {
    let repo = init_repo(
        "workspace-version-bump",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    add_workspace_crate(repo.path(), "support-lib");
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    replace_in_file(
        &repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"0.1.1\"",
    );
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    let primary_changelog = format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()));
    assert!(
        output.contains(&primary_changelog),
        "missing primary crate changelog in output:\n{output}"
    );
    assert!(
        output.contains("crates/support-lib/CHANGELOG.md"),
        "missing secondary crate changelog in output:\n{output}"
    );
}

#[test]
fn generated_repo_check_rejects_crate_layout_config_drift() {
    let repo = init_repo(
        "crate-config-drift",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    add_workspace_crate(repo.path(), "support-lib");
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    replace_in_file(
        &repo.path().join("repo-check.toml"),
        &format!(
            "package_manifest_path = \"crates/{}/Cargo.toml\"",
            repo_slug(repo.path())
        ),
        "package_manifest_path = \"crates/support-lib/Cargo.toml\"",
    );
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("crate layout config drift"),
        "expected config drift validation, got:\n{output}"
    );
}

#[test]
fn generated_repo_check_allows_deleting_a_crate_with_its_changelog() {
    let repo = init_repo(
        "crate-delete-legal",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());
    add_workspace_crate(repo.path(), "support-lib");
    git_commit_all(repo.path(), "chore(repo): initial scaffold");

    fs::remove_dir_all(repo.path().join("crates/support-lib"))
        .expect("failed to delete secondary crate");
    remove_workspace_member(repo.path(), "support-lib");
    append_to_file(
        &repo
            .path()
            .join(format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()))),
        "\n- note remove support-lib crate\n",
    );
    git_add_all(repo.path());

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn generated_repo_check_rejects_major_version_downgrade() {
    let repo = init_repo("node-major-downgrade", &["--project", "nodejs"]);
    git_init(repo.path());
    git_config_identity(repo.path());

    replace_in_file(
        &repo.path().join("package.json"),
        "\"version\": \"0.1.0\"",
        "\"version\": \"2.0.0\"",
    );
    git_commit_all(repo.path(), "chore(repo): prepare major downgrade baseline");

    replace_in_file(
        &repo.path().join("package.json"),
        "\"version\": \"2.0.0\"",
        "\"version\": \"1.0.0\"",
    );
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("refusing major version change by default"),
        "expected major downgrade gate, got:\n{output}"
    );
}

#[test]
fn generated_repo_check_detects_major_bump_for_workspace_table_inheritance() {
    let repo = init_repo(
        "workspace-table-major",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    git_config_identity(repo.path());

    replace_in_file(
        &repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0-alpha.1\"",
    );
    replace_in_file(
        &repo
            .path()
            .join("crates")
            .join(repo_slug(repo.path()))
            .join("Cargo.toml"),
        "version.workspace = true",
        "version = { workspace = true }",
    );
    git_commit_all(
        repo.path(),
        "chore(repo): prepare workspace version baseline",
    );

    replace_in_file(
        &repo.path().join("Cargo.toml"),
        "version = \"1.0.0-alpha.1\"",
        "version = \"2.0.0-alpha.1\"",
    );
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("refusing major version change by default"),
        "expected workspace-inherited major bump gate, got:\n{output}"
    );
    assert!(
        !output.contains("unsupported version"),
        "workspace table form or prerelease parsing was rejected:\n{output}"
    );
}

#[test]
fn generated_python_repo_check_accepts_prerelease_major_versions() {
    let repo = init_repo("python-prerelease-major", &["--project", "python"]);
    git_init(repo.path());
    git_config_identity(repo.path());

    replace_in_file(
        &repo.path().join("pyproject.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0rc1\"",
    );
    git_commit_all(
        repo.path(),
        "chore(repo): prepare python prerelease baseline",
    );

    replace_in_file(
        &repo.path().join("pyproject.toml"),
        "version = \"1.0.0rc1\"",
        "version = \"2.0.0rc1\"",
    );
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("refusing major version change by default"),
        "expected python prerelease major bump gate, got:\n{output}"
    );
    assert!(
        !output.contains("unsupported version"),
        "python prerelease version was rejected:\n{output}"
    );
}

#[test]
fn generated_node_repo_check_uses_top_level_prerelease_version() {
    let repo = init_repo("node-top-level-version", &["--project", "nodejs"]);
    git_init(repo.path());
    git_config_identity(repo.path());

    fs::write(
        repo.path().join("package.json"),
        concat!(
            "{\n",
            "  \"name\": \"node-top-level-version\",\n",
            "  \"publishConfig\": { \"version\": \"9.9.9\" },\n",
            "  \"version\": \"1.0.0-beta.1\",\n",
            "  \"type\": \"module\"\n",
            "}\n"
        ),
    )
    .expect("failed to write baseline package.json");
    git_commit_all(repo.path(), "chore(repo): prepare node prerelease baseline");

    fs::write(
        repo.path().join("package.json"),
        concat!(
            "{\n",
            "  \"name\": \"node-top-level-version\",\n",
            "  \"publishConfig\": { \"version\": \"1.0.0-beta.1\" },\n",
            "  \"version\": \"2.0.0-beta.1\",\n",
            "  \"type\": \"module\"\n",
            "}\n"
        ),
    )
    .expect("failed to write updated package.json");
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("refusing major version change by default"),
        "expected node top-level major bump gate, got:\n{output}"
    );
    assert!(
        !output.contains("unsupported version"),
        "node prerelease version was rejected:\n{output}"
    );
}

#[test]
fn generated_node_repo_check_uses_top_level_version_in_compact_package_json() {
    let repo = init_repo("node-compact-version", &["--project", "nodejs"]);
    git_init(repo.path());
    git_config_identity(repo.path());

    fs::write(
        repo.path().join("package.json"),
        "{\"name\":\"node-compact-version\",\"publishConfig\":{\"version\":\"9.9.9\"},\"version\":\"1.0.0-beta.1\",\"type\":\"module\"}\n",
    )
    .expect("failed to write compact baseline package.json");
    git_commit_all(repo.path(), "chore(repo): prepare compact node baseline");

    fs::write(
        repo.path().join("package.json"),
        "{\"name\":\"node-compact-version\",\"publishConfig\":{\"version\":\"1.0.0-beta.1\"},\"version\":\"2.0.0-beta.1\",\"type\":\"module\"}\n",
    )
    .expect("failed to write compact updated package.json");
    git_add_all(repo.path());

    let output = run_generated_repo_check_failure(repo.path(), &["pre-commit"]);
    assert!(
        output.contains("refusing major version change by default"),
        "expected compact package.json major bump gate, got:\n{output}"
    );
    assert!(
        !output.contains("unsupported version"),
        "compact package.json prerelease version was rejected:\n{output}"
    );
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

fn run_cli_failure<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut command = Command::new(cli_binary());
    for arg in args {
        command.arg(arg.as_ref());
    }
    run_fail("omne-project-init", &mut command)
}

fn run_generated_repo_check(repo_root: &Path, args: &[&str]) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifest_path = repo_root.join("tools/repo-check/Cargo.toml");
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--target-dir")
        .arg(generated_target_dir(repo_root))
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

    run_ok("generated repo-check", &mut command)
}

fn run_generated_repo_check_failure(repo_root: &Path, args: &[&str]) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifest_path = repo_root.join("tools/repo-check/Cargo.toml");
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--target-dir")
        .arg(generated_target_dir(repo_root))
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

    run_fail("generated repo-check", &mut command)
}

fn cargo_metadata(repo_root: &Path) -> String {
    run_ok(
        "cargo metadata",
        Command::new("cargo")
            .arg("metadata")
            .arg("--format-version")
            .arg("1")
            .arg("--no-deps")
            .current_dir(repo_root),
    )
}

fn git_init(repo_root: &Path) {
    let init_with_branch = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("init")
        .arg("-b")
        .arg("main")
        .output()
        .expect("failed to execute `git init -b main`");

    if init_with_branch.status.success() {
        return;
    }

    run_ok(
        "git init",
        Command::new("git").arg("-C").arg(repo_root).arg("init"),
    );
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("branch")
        .arg("-m")
        .arg("main")
        .output()
        .expect("failed to execute `git branch -m main`");
}

fn git_config_identity(repo_root: &Path) {
    run_ok(
        "git config user.name",
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["config", "user.name", "Smoke Test"]),
    );
    run_ok(
        "git config user.email",
        Command::new("git").arg("-C").arg(repo_root).args([
            "config",
            "user.email",
            "smoke@example.com",
        ]),
    );
}

fn git_add_all(repo_root: &Path) {
    run_ok(
        "git add -A",
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["add", "-A"]),
    );
}

fn git_commit_all(repo_root: &Path, message: &str) {
    git_add_all(repo_root);
    run_ok(
        "git commit",
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["commit", "-m", message]),
    );
}

fn replace_in_file(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let replaced = text.replace(from, to);
    assert_ne!(
        text,
        replaced,
        "replace_in_file did not find expected text in {}",
        path.display()
    );
    fs::write(path, replaced)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn append_to_file(path: &Path, suffix: &str) {
    let mut text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    text.push_str(suffix);
    fs::write(path, text)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

fn add_workspace_crate(repo_root: &Path, crate_name: &str) {
    let source = repo_root.join("crates").join(repo_slug(repo_root));
    let destination = repo_root.join("crates").join(crate_name);
    copy_dir_all(&source, &destination);

    let source_slug = repo_slug(repo_root).to_string();
    replace_in_file(&destination.join("Cargo.toml"), &source_slug, crate_name);
    replace_in_file(&destination.join("CHANGELOG.md"), &source_slug, crate_name);
    append_workspace_member(repo_root, crate_name);
}

fn append_workspace_member(repo_root: &Path, crate_name: &str) {
    let cargo_toml = repo_root.join("Cargo.toml");
    let text = fs::read_to_string(&cargo_toml)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", cargo_toml.display()));
    let insert_after = "members = [";
    let marker = format!("\"crates/{crate_name}\"");
    assert!(
        text.contains(insert_after),
        "workspace members array not found in {}",
        cargo_toml.display()
    );
    if text.contains(&marker) {
        return;
    }

    let updated = text.replacen(
        insert_after,
        &format!("{insert_after}\"crates/{crate_name}\", "),
        1,
    );
    fs::write(&cargo_toml, updated)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", cargo_toml.display()));
}

fn remove_workspace_member(repo_root: &Path, crate_name: &str) {
    let cargo_toml = repo_root.join("Cargo.toml");
    let text = fs::read_to_string(&cargo_toml)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", cargo_toml.display()));
    let member = format!("\"crates/{crate_name}\", ");
    assert!(
        text.contains(&member),
        "workspace member {member} not found in {}",
        cargo_toml.display()
    );
    let updated = text.replacen(&member, "", 1);
    fs::write(&cargo_toml, updated)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", cargo_toml.display()));
}

fn copy_dir_all(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap_or_else(|error| {
        panic!(
            "failed to create copy destination {}: {error}",
            destination.display()
        )
    });

    for entry in fs::read_dir(source)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", source.display()))
    {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read entry in {}: {error}", source.display())
        });
        let entry_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry_path.is_dir() {
            copy_dir_all(&entry_path, &destination_path);
        } else {
            fs::copy(&entry_path, &destination_path).unwrap_or_else(|error| {
                panic!(
                    "failed to copy {} -> {}: {error}",
                    entry_path.display(),
                    destination_path.display()
                )
            });
        }
    }
}

#[derive(Clone, Copy)]
enum ProjectKind {
    Rust,
    Python,
    Nodejs,
}

impl ProjectKind {
    fn template_dir(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Nodejs => "nodejs",
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

#[derive(Clone, Copy)]
enum Layout {
    Root,
    Crate,
}

impl Layout {
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

#[derive(Clone, Copy)]
struct ManifestSpec {
    project_kind: ProjectKind,
    layout: Layout,
    repo_name: &'static str,
    package_name: &'static str,
    crate_dir: &'static str,
    python_package: &'static str,
}

impl ManifestSpec {
    fn changelog_path(self) -> String {
        match self.layout {
            Layout::Root => "CHANGELOG.md".to_string(),
            Layout::Crate => format!("crates/{}/CHANGELOG.md", self.crate_dir),
        }
    }

    fn package_manifest_path(self) -> String {
        match (self.project_kind, self.layout) {
            (ProjectKind::Rust, Layout::Root) => "Cargo.toml".to_string(),
            (ProjectKind::Rust, Layout::Crate) => format!("crates/{}/Cargo.toml", self.crate_dir),
            (ProjectKind::Python, _) => "pyproject.toml".to_string(),
            (ProjectKind::Nodejs, _) => "package.json".to_string(),
        }
    }

    fn primary_source_path(self) -> String {
        match (self.project_kind, self.layout) {
            (ProjectKind::Rust, Layout::Root) => "src/main.rs".to_string(),
            (ProjectKind::Rust, Layout::Crate) => {
                format!("crates/{}/src/lib.rs", self.crate_dir)
            }
            (ProjectKind::Python, _) => format!("{}/__init__.py", self.python_package),
            (ProjectKind::Nodejs, _) => "src/index.js".to_string(),
        }
    }
}

fn assert_manifest_matches_templates(output: &str, spec: ManifestSpec) {
    let actual = parse_manifest_output(output);
    let expected = expected_manifest_entries(spec);
    assert_eq!(
        actual, expected,
        "manifest output drifted from tracked template set"
    );
}

fn parse_manifest_output(output: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line == "[" || line == "]" || line.is_empty() {
            continue;
        }
        let value = line
            .trim_end_matches(',')
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or_else(|| panic!("invalid manifest entry line: {line}"));
        entries.push(value.to_string());
    }
    entries
}

fn expected_manifest_entries(spec: ManifestSpec) -> Vec<String> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let common_prefix = PathBuf::from("templates/common");
    let project_prefix = PathBuf::from(format!(
        "templates/projects/{}/{}",
        spec.project_kind.template_dir(),
        spec.layout.as_str()
    ));
    let tracked_paths = git_ls_files(
        &repo_root,
        &[common_prefix.as_path(), project_prefix.as_path()],
    );

    let mut entries = BTreeSet::new();
    for tracked_path in tracked_paths {
        let relative_path = tracked_path
            .strip_prefix(&common_prefix)
            .or_else(|_| tracked_path.strip_prefix(&project_prefix))
            .unwrap_or_else(|_| {
                panic!(
                    "tracked template path {} was outside expected prefixes",
                    tracked_path.display()
                )
            });
        entries.insert(render_template_path(&relative_path.to_string_lossy(), spec));
    }

    entries.into_iter().collect()
}

fn render_template_path(value: &str, spec: ManifestSpec) -> String {
    value
        .replace("__TEMPLATE_VERSION__", "0.1.0")
        .replace("__REPO_CHECK_SCHEMA_VERSION__", "1")
        .replace("__REPO_NAME__", spec.repo_name)
        .replace("__PACKAGE_NAME__", spec.package_name)
        .replace("__CRATE_DIR__", spec.crate_dir)
        .replace("__PY_PACKAGE__", spec.python_package)
        .replace("__PROJECT_KIND__", spec.project_kind.as_str())
        .replace("__LAYOUT__", spec.layout.as_str())
        .replace("__LAYOUT_LABEL__", spec.layout.label())
        .replace("__CHANGELOG_PATH__", &spec.changelog_path())
        .replace("__PACKAGE_MANIFEST_PATH__", &spec.package_manifest_path())
        .replace("__PRIMARY_SOURCE_PATH__", &spec.primary_source_path())
        .replace(
            "__PRIMARY_VALIDATION_COMMAND__",
            "cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local",
        )
}

fn git_ls_files(repo_root: &Path, pathspecs: &[&Path]) -> Vec<PathBuf> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .arg("-z")
        .arg("--");
    for pathspec in pathspecs {
        command.arg(pathspec);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("git ls-files failed: {error}"));
    assert!(
        output.status.success(),
        "git ls-files failed:\n{}",
        render_output(&output)
    );

    let stdout = String::from_utf8(output.stdout).expect("git ls-files stdout must be utf-8");
    stdout
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect()
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

fn run_fail(label: &str, command: &mut Command) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: failed to execute command: {error}"));
    assert!(
        !output.status.success(),
        "{label}: command unexpectedly succeeded\n{}",
        render_output(&output)
    );
    render_output(&output)
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

fn repo_slug(repo_root: &Path) -> &str {
    repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .expect("temp repo path must end with a UTF-8 file name")
}

fn generated_target_dir(_repo_root: &Path) -> PathBuf {
    static TARGET_ROOT: OnceLock<PathBuf> = OnceLock::new();
    TARGET_ROOT
        .get_or_init(|| {
            let path = env::temp_dir().join("omne-project-init-generated-target");
            fs::create_dir_all(&path).expect("failed to create generated target root");
            path
        })
        .clone()
}

fn repo_template_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn template_fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn generated_repo_check_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn has_supported_python() -> bool {
    python_command_version(&["python"]).is_some_and(|version| version >= (3, 11))
        || python_command_version(&["python3"]).is_some_and(|version| version >= (3, 11))
        || python_command_version(&["py", "-3"]).is_some_and(|version| version >= (3, 11))
}

fn python_command_version(command: &[&str]) -> Option<(u64, u64)> {
    let (program, args) = command.split_first()?;
    let mut process = Command::new(program);
    process.args(args).args([
        "-c",
        "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}')",
    ]);
    let output = process.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let (major, minor) = stdout.trim().split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn command_works(program: &str, args: &[&str]) -> bool {
    matches!(
        Command::new(program).args(args).output(),
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
