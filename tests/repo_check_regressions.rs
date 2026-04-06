use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
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
fn workspace_local_rejects_rust_clippy_warnings() {
    let repo = init_repo(
        "rust-workspace-local-clippy",
        &["--project", "rust", "--layout", "root"],
    );

    inject_unused_variable_warning(repo.path());

    let error = run_generated_repo_check_fail(repo.path(), &["workspace", "local"]);
    assert!(
        error.contains("rust clippy"),
        "expected workspace local to run clippy, got: {error}"
    );
    assert!(
        error.contains("unused variable"),
        "expected clippy warning details, got: {error}"
    );
}

#[test]
fn documented_workspace_local_does_not_dirty_rust_repos() {
    for layout in ["root", "crate"] {
        let repo = init_repo(
            &format!("rust-workspace-clean-{layout}"),
            &["--project", "rust", "--layout", layout],
        );
        git_init(repo.path());
        commit_all(repo.path(), "feat(repo): initial scaffold");

        let cargo_lock = repo.path().join("Cargo.lock");
        assert!(
            cargo_lock.is_file(),
            "expected generated Cargo.lock for {layout}"
        );
        let original_lock = fs::read_to_string(&cargo_lock).expect("read Cargo.lock");

        run_documented_repo_check_command(repo.path(), &["workspace", "local"]);

        assert!(
            cargo_lock.is_file(),
            "workspace local removed Cargo.lock for {layout}"
        );
        assert_eq!(
            fs::read_to_string(&cargo_lock).expect("read Cargo.lock after workspace local"),
            original_lock,
            "workspace local rewrote Cargo.lock for {layout}"
        );
        assert_eq!(
            git_status_short(repo.path()).trim(),
            "",
            "workspace local dirtied the repo for {layout}"
        );
    }
}

#[test]
fn pre_commit_rejects_rust_clippy_warnings() {
    let repo = init_repo(
        "rust-pre-commit-clippy",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    inject_unused_variable_warning(repo.path());
    append_to_file(
        &repo.path().join("CHANGELOG.md"),
        "- keep rust clippy gate aligned with pre-commit\n",
    );
    git_add(repo.path(), &["src/main.rs", "CHANGELOG.md"]);

    let error = run_generated_repo_check_fail(repo.path(), &["pre-commit"]);
    assert!(
        error.contains("rust clippy"),
        "expected pre-commit to run clippy, got: {error}"
    );
    assert!(
        error.contains("unused variable"),
        "expected clippy warning details, got: {error}"
    );
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
fn workspace_local_validates_declared_python_requires_python() {
    let repo = init_repo("python-requires-python", &["--project", "python"]);

    replace_in_file(
        repo.path().join("pyproject.toml"),
        "requires-python = \">=3.11\"",
        "requires-python = \">=99.0\"",
    );

    let error = run_generated_repo_check_fail(repo.path(), &["workspace", "local"]);
    assert!(
        error.contains("requires-python"),
        "expected requires-python failure, got: {error}"
    );
    assert!(
        error.contains(">=99.0"),
        "expected declared python contract in failure, got: {error}"
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
fn commit_msg_detects_single_line_node_major_bump() {
    let repo = init_repo("node-major-bump-single-line", &["--project", "nodejs"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::write(
        repo.path().join("package.json"),
        "{\"version\":\"1.2.3\",\"meta\":{\"version\":\"0.0.1\"},\"name\":\"node-major-bump-single-line\",\"type\":\"module\",\"scripts\":{\"test\":\"node --test\"}}\n",
    )
    .expect("write package.json");
    git_add(repo.path(), &["package.json"]);
    commit_all(repo.path(), "chore(node): prepare major baseline");

    fs::write(
        repo.path().join("package.json"),
        "{\"version\":\"2.0.0\",\"meta\":{\"version\":\"0.0.1\"},\"name\":\"node-major-bump-single-line\",\"type\":\"module\",\"scripts\":{\"test\":\"node --test\"}}\n",
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
fn pre_commit_requires_override_for_stable_major_transition() {
    let repo = init_repo(
        "rust-stable-major-pre-commit",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0\"",
    );
    append_to_file(&repo.path().join("CHANGELOG.md"), "- enter stable major\n");
    git_add(repo.path(), &["Cargo.toml", "CHANGELOG.md"]);

    let error = run_generated_repo_check_fail(repo.path(), &["pre-commit"]);
    assert!(
        error.contains("refusing major version change by default"),
        "expected stable major transition to require override, got: {error}"
    );

    run_generated_repo_check_with_env(
        repo.path(),
        &[("OMNE_ALLOW_MAJOR_VERSION_BUMP", "1")],
        &["pre-commit"],
    );
}

#[test]
fn initial_stable_release_is_not_treated_as_major_bump() {
    let repo = init_repo(
        "rust-stable-major-initial",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0\"",
    );
    run_git(repo.path(), &["add", "."]);

    run_generated_repo_check(repo.path(), &["pre-commit"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.initial");
    fs::write(&commit_msg, "feat(repo): initial stable release\n").expect("write commit msg");
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
fn commit_msg_requires_breaking_marker_for_stable_major_transition() {
    let repo = init_repo(
        "rust-stable-major-commit-msg",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0\"",
    );
    git_add(repo.path(), &["Cargo.toml"]);

    let plain_commit_msg = repo.path().join("COMMIT_EDITMSG.plain");
    fs::write(&plain_commit_msg, "feat(repo): enter stable major\n").expect("write commit msg");
    let error = run_generated_repo_check_fail(
        repo.path(),
        &[
            "commit-msg",
            "--commit-msg-file",
            plain_commit_msg.to_string_lossy().as_ref(),
        ],
    );
    assert!(
        error.contains("requires an explicit breaking commit message"),
        "expected stable major transition to require breaking marker, got: {error}"
    );

    let breaking_commit_msg = repo.path().join("COMMIT_EDITMSG.breaking");
    fs::write(&breaking_commit_msg, "feat(repo)!: enter stable major\n")
        .expect("write breaking commit msg");
    run_generated_repo_check(
        repo.path(),
        &[
            "commit-msg",
            "--commit-msg-file",
            breaking_commit_msg.to_string_lossy().as_ref(),
        ],
    );
}

#[test]
fn commit_msg_accepts_breaking_change_footer_for_major_bump() {
    let repo = init_repo(
        "rust-breaking-footer",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0\"",
    );
    git_add(repo.path(), &["Cargo.toml"]);
    commit_all(repo.path(), "feat(repo)!: enter stable major");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"1.0.0\"",
        "version = \"2.0.0\"",
    );
    git_add(repo.path(), &["Cargo.toml"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.test");
    fs::write(
        &commit_msg,
        "refactor(repo): prepare next stable major\n\nBREAKING CHANGE: regenerate callers against the new layout\n",
    )
    .expect("write commit msg");
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
fn prerelease_versions_are_accepted_by_commit_msg_gate() {
    let repo = init_repo(
        "rust-prerelease",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0-alpha.1\"",
    );
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
    let repo = init_repo(
        "root-config-changelog",
        &["--project", "rust", "--layout", "root"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    fs::create_dir_all(repo.path().join("docs")).expect("create docs dir");
    let changelog_text =
        fs::read_to_string(repo.path().join("CHANGELOG.md")).expect("read original changelog");
    fs::write(repo.path().join("docs/CHANGELOG.md"), changelog_text)
        .expect("write moved changelog");
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
        &[
            "repo-check.toml",
            "docs/CHANGELOG.md",
            "CHANGELOG.md",
            "src/main.rs",
        ],
    );

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn workspace_version_bump_requires_each_inheriting_crate_changelog() {
    let repo = init_repo(
        "crate-workspace-version-changelog",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    add_workspace_crate(repo.path(), "support-lib");
    run_git(repo.path(), &["add", "-A"]);
    commit_all(repo.path(), "feat(repo): add support crate");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"0.2.0\"",
    );
    git_add(repo.path(), &["Cargo.toml"]);

    let error = run_generated_repo_check_fail(repo.path(), &["pre-commit"]);
    let primary_changelog = format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()));
    assert!(
        error.contains(&primary_changelog),
        "expected primary crate changelog requirement, got: {error}"
    );
    assert!(
        error.contains("crates/support-lib/CHANGELOG.md"),
        "expected inherited workspace version to require support-lib changelog, got: {error}"
    );
}

#[test]
fn retiring_a_crate_allows_deleting_its_changelog() {
    let repo = init_repo(
        "crate-retirement-changelog",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    add_workspace_crate(repo.path(), "support-lib");
    run_git(repo.path(), &["add", "-A"]);
    commit_all(repo.path(), "feat(repo): add support crate");

    fs::remove_dir_all(repo.path().join("crates/support-lib")).expect("remove support-lib crate");
    replace_in_file(
        repo.path().join("Cargo.toml"),
        "\"crates/support-lib\", ",
        "",
    );
    append_to_file(
        &repo
            .path()
            .join(format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()))),
        "\n- retire support-lib crate\n",
    );
    run_git(repo.path(), &["add", "-A"]);

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn crate_layout_root_governance_changes_require_primary_changelog() {
    let repo = init_repo(
        "crate-root-changelog",
        &["--project", "rust", "--layout", "crate"],
    );
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
fn crate_layout_allows_renaming_primary_crate_dir() {
    let repo = init_repo("crate-rename", &["--project", "rust", "--layout", "crate"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    let old_crate_dir = repo_slug(repo.path()).to_string();
    let new_crate_dir = format!("{old_crate_dir}-renamed");
    fs::rename(
        repo.path().join(format!("crates/{old_crate_dir}")),
        repo.path().join(format!("crates/{new_crate_dir}")),
    )
    .expect("rename primary crate dir");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        &format!("\"crates/{old_crate_dir}\""),
        &format!("\"crates/{new_crate_dir}\""),
    );
    replace_in_file(
        repo.path().join("repo-check.toml"),
        &format!("crate_dir = \"{old_crate_dir}\""),
        &format!("crate_dir = \"{new_crate_dir}\""),
    );
    replace_in_file(
        repo.path().join("repo-check.toml"),
        &format!("package_manifest_path = \"crates/{old_crate_dir}/Cargo.toml\""),
        &format!("package_manifest_path = \"crates/{new_crate_dir}/Cargo.toml\""),
    );
    replace_in_file(
        repo.path().join("repo-check.toml"),
        &format!("changelog_path = \"crates/{old_crate_dir}/CHANGELOG.md\""),
        &format!("changelog_path = \"crates/{new_crate_dir}/CHANGELOG.md\""),
    );

    let lib_rs = repo
        .path()
        .join(format!("crates/{new_crate_dir}/src/lib.rs"));
    let mut lib_text = fs::read_to_string(&lib_rs).expect("read renamed lib.rs");
    lib_text.push_str("\n// keep crate rename regression covered\n");
    fs::write(&lib_rs, lib_text).expect("write renamed lib.rs");

    let mut changelog = fs::read_to_string(
        repo.path()
            .join(format!("crates/{new_crate_dir}/CHANGELOG.md")),
    )
    .expect("read renamed changelog");
    changelog.push_str("- rename primary crate directory\n");
    fs::write(
        repo.path()
            .join(format!("crates/{new_crate_dir}/CHANGELOG.md")),
        changelog,
    )
    .expect("write renamed changelog");

    let mut readme = fs::read_to_string(repo.path().join("README.md")).expect("read README");
    readme.push_str("\ncrate rename regression\n");
    fs::write(repo.path().join("README.md"), readme).expect("write README");

    run_git(repo.path(), &["add", "-A"]);
    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn deleting_nested_crate_docs_changelog_is_not_treated_as_package_changelog() {
    let repo = init_repo(
        "crate-nested-docs-changelog",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    let crate_dir = repo_slug(repo.path());
    let nested_changelog = format!("crates/{crate_dir}/docs/CHANGELOG.md");
    fs::create_dir_all(repo.path().join(format!("crates/{crate_dir}/docs")))
        .expect("create docs dir");
    fs::write(
        repo.path().join(&nested_changelog),
        "# nested changelog doc\n",
    )
    .expect("write nested changelog");
    git_add(repo.path(), &[nested_changelog.as_str()]);
    commit_all(repo.path(), "docs(crate): add nested changelog doc");

    fs::remove_file(repo.path().join(&nested_changelog)).expect("remove nested changelog");
    let lib_rs = repo.path().join(format!("crates/{crate_dir}/src/lib.rs"));
    let mut lib_text = fs::read_to_string(&lib_rs).expect("read lib.rs");
    lib_text.push_str("\n// nested docs changelog cleanup regression\n");
    fs::write(&lib_rs, lib_text).expect("write lib.rs");
    let primary_changelog = format!("crates/{crate_dir}/CHANGELOG.md");
    let mut changelog =
        fs::read_to_string(repo.path().join(&primary_changelog)).expect("read primary changelog");
    changelog.push_str("- note nested docs changelog cleanup\n");
    fs::write(repo.path().join(&primary_changelog), changelog).expect("write primary changelog");
    git_add(
        repo.path(),
        &[
            nested_changelog.as_str(),
            primary_changelog.as_str(),
            &format!("crates/{crate_dir}/src/lib.rs"),
        ],
    );

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn hook_templates_recognize_windows_absolute_manifest_paths() {
    let pre_commit = fs::read_to_string("templates/common/githooks/pre-commit")
        .expect("read pre-commit hook template");
    let commit_msg = fs::read_to_string("templates/common/githooks/commit-msg")
        .expect("read commit-msg hook template");

    for text in [&pre_commit, &commit_msg] {
        assert!(
            text.contains("[A-Za-z]:/*"),
            "missing drive-letter path detection"
        );
        assert!(
            text.contains("[A-Za-z]:\\\\*"),
            "missing backslash path detection"
        );
        assert!(text.contains("\\\\\\\\*"), "missing UNC path detection");
    }
}

#[test]
fn workspace_local_resolves_repo_root_from_crate_subdir_without_git() {
    let repo = init_repo(
        "subdir-workspace-local-no-git",
        &["--project", "rust", "--layout", "crate"],
    );
    let crate_dir = repo_slug(repo.path());
    let current_dir = repo.path().join(format!("crates/{crate_dir}"));

    let output =
        run_generated_repo_check_from_dir(&current_dir, repo.path(), &["workspace", "local"]);
    assert!(
        output.contains("running Local checks"),
        "expected workspace local to resolve the repo root without git metadata, got: {output}"
    );
}

#[test]
fn repo_check_config_accepts_single_quoted_toml_strings() {
    let repo = init_repo("single-quoted-config", &["--project", "nodejs"]);
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    let config_path = repo.path().join("repo-check.toml");
    let config_text = fs::read_to_string(&config_path).expect("read repo-check.toml");
    fs::write(&config_path, config_text.replace('"', "'")).expect("write repo-check.toml");

    append_to_file(
        &repo.path().join("CHANGELOG.md"),
        "- keep single-quoted repo-check config covered\n",
    );
    git_add(repo.path(), &["repo-check.toml", "CHANGELOG.md"]);

    run_generated_repo_check(repo.path(), &["pre-commit"]);
}

#[test]
fn commit_msg_uses_configured_nested_workspace_manifest_path() {
    let repo = init_repo(
        "nested-workspace-config",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    let crate_dir = repo_slug(repo.path()).to_string();
    fs::create_dir_all(repo.path().join("workspace/crates")).expect("create nested workspace");
    fs::rename(
        repo.path().join("Cargo.toml"),
        repo.path().join("workspace/Cargo.toml"),
    )
    .expect("move workspace manifest");
    fs::rename(
        repo.path().join(format!("crates/{crate_dir}")),
        repo.path().join(format!("workspace/crates/{crate_dir}")),
    )
    .expect("move primary crate into nested workspace");
    fs::remove_dir(repo.path().join("crates")).expect("remove empty crates dir");

    replace_in_file(
        repo.path().join("repo-check.toml"),
        &format!("package_manifest_path = \"crates/{crate_dir}/Cargo.toml\""),
        &format!("package_manifest_path = \"workspace/crates/{crate_dir}/Cargo.toml\""),
    );
    replace_in_file(
        repo.path().join("repo-check.toml"),
        &format!("changelog_path = \"crates/{crate_dir}/CHANGELOG.md\""),
        &format!("changelog_path = \"workspace/crates/{crate_dir}/CHANGELOG.md\""),
    );
    run_git(repo.path(), &["add", "-A"]);
    commit_all(
        repo.path(),
        "refactor(repo): move workspace manifest under workspace",
    );

    replace_in_file(
        repo.path().join("workspace/Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0\"",
    );
    git_add(repo.path(), &["workspace/Cargo.toml"]);

    let commit_msg = repo.path().join("COMMIT_EDITMSG.nested-workspace");
    fs::write(&commit_msg, "feat(repo): stable major without marker\n").expect("write commit msg");
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
        "expected nested workspace manifest path to drive major bump detection, got: {error}"
    );
}

#[test]
fn workspace_local_accepts_running_from_a_subdirectory_without_repo_root_override() {
    let repo = init_repo(
        "subdir-workspace-local",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    let nested = repo.path().join("subdir").join("nested");
    fs::create_dir_all(&nested).expect("create nested subdir");

    let output = run_generated_repo_check_from_dir(&nested, repo.path(), &["workspace", "local"]);
    assert!(
        output.contains("running Local checks"),
        "expected workspace local to resolve the repo root from a subdirectory, got: {output}"
    );
}

#[test]
fn install_hooks_accepts_running_from_a_subdirectory_without_repo_root_override() {
    let repo = init_repo(
        "subdir-install-hooks",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());

    let nested = repo.path().join("subdir").join("nested");
    fs::create_dir_all(&nested).expect("create nested subdir");

    let output = run_generated_repo_check_from_dir(&nested, repo.path(), &["install-hooks"]);
    assert!(
        output.contains("Configured git hooks"),
        "expected install-hooks to resolve the repo root from a subdirectory, got: {output}"
    );
}

#[test]
fn adding_a_new_nonzero_major_workspace_crate_is_not_treated_as_a_major_bump() {
    let repo = init_repo(
        "crate-add-major",
        &["--project", "rust", "--layout", "crate"],
    );
    git_init(repo.path());
    commit_all(repo.path(), "feat(repo): initial scaffold");

    replace_in_file(
        repo.path().join("Cargo.toml"),
        "version = \"0.1.0\"",
        "version = \"1.0.0\"",
    );
    run_git(repo.path(), &["add", "Cargo.toml"]);
    run_git(
        repo.path(),
        &["commit", "-m", "feat(repo)!: enter stable major"],
    );

    add_workspace_crate(repo.path(), "support-lib");
    let primary_changelog = format!("crates/{}/CHANGELOG.md", repo_slug(repo.path()));
    append_to_file(
        &repo.path().join(&primary_changelog),
        "\n- add support-lib crate\n",
    );
    append_to_file(
        &repo.path().join("crates/support-lib/CHANGELOG.md"),
        "\n- add support-lib crate\n",
    );
    run_git(repo.path(), &["add", "-A"]);

    let output = run_generated_repo_check(repo.path(), &["pre-commit"]);
    assert!(
        !output.contains("major version change"),
        "adding a new crate should not require major bump override:\n{output}"
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
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = generated_repo_check_command(repo_root, args, true);
    run_ok("generated repo-check", &mut command)
}

fn run_generated_repo_check_with_env(
    repo_root: &Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = generated_repo_check_command(repo_root, args, true);
    for (key, value) in envs {
        command.env(key, value);
    }
    run_ok("generated repo-check", &mut command)
}

fn run_generated_repo_check_fail(repo_root: &Path, args: &[&str]) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = generated_repo_check_command(repo_root, args, true);
    run_fail("generated repo-check", &mut command)
}

fn run_generated_repo_check_from_dir(
    current_dir: &Path,
    repo_root: &Path,
    args: &[&str],
) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = generated_repo_check_command(repo_root, args, false);
    command.current_dir(current_dir);
    run_ok("generated repo-check", &mut command)
}

fn generated_repo_check_command(repo_root: &Path, args: &[&str], add_repo_root: bool) -> Command {
    let manifest_path = repo_root.join("tools/repo-check/Cargo.toml");
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--target-dir")
        .arg(generated_target_dir())
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
    if add_repo_root && !saw_repo_root {
        command.arg("--repo-root").arg(repo_root);
    }
    command
}

fn run_documented_repo_check_command(repo_root: &Path, args: &[&str]) -> String {
    let _guard = generated_repo_check_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(repo_root.join("tools/repo-check/Cargo.toml"))
        .arg("--");
    for arg in args {
        command.arg(arg);
    }
    command.arg("--repo-root").arg(repo_root);
    run_ok("documented repo-check", &mut command)
}

fn generated_repo_check_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn generated_target_dir() -> PathBuf {
    static TARGET_ROOT: OnceLock<PathBuf> = OnceLock::new();
    TARGET_ROOT
        .get_or_init(|| {
            let path = env::temp_dir().join("omne-project-init-generated-target");
            fs::create_dir_all(&path).expect("failed to create generated target root");
            path
        })
        .clone()
}

fn replace_in_file(path: PathBuf, from: &str, to: &str) {
    let text = fs::read_to_string(&path).expect("read file for replacement");
    let updated = text.replace(from, to);
    assert_ne!(
        text,
        updated,
        "replacement target not found in {}",
        path.display()
    );
    fs::write(&path, updated).expect("write file after replacement");
}

fn append_to_file(path: &Path, suffix: &str) {
    let mut text = fs::read_to_string(path).expect("read file for append");
    text.push_str(suffix);
    fs::write(path, text).expect("write appended file");
}

fn inject_unused_variable_warning(repo_root: &Path) {
    replace_in_file(
        repo_root.join("src/main.rs"),
        "fn main() {\n    println!(\"",
        "fn main() {\n    let unused = 1;\n    println!(\"",
    );
}

fn add_workspace_crate(repo_root: &Path, crate_name: &str) {
    let source = repo_root.join("crates").join(repo_slug(repo_root));
    let destination = repo_root.join("crates").join(crate_name);
    copy_dir_all(&source, &destination);
    replace_in_file(
        destination.join("Cargo.toml"),
        repo_slug(repo_root),
        crate_name,
    );
    replace_in_file(
        repo_root.join("Cargo.toml"),
        "\"crates/",
        &format!("\"crates/{crate_name}\", \"crates/"),
    );
}

fn copy_dir_all(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copy destination");
    for entry in fs::read_dir(source).expect("read copy source directory") {
        let entry = entry.expect("read copy source entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).unwrap_or_else(|error| {
                panic!(
                    "failed to copy {} -> {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            });
        }
    }
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

fn git_status_short(repo_root: &Path) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(["status", "--short"]);
    run_ok("git status --short", &mut command)
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
