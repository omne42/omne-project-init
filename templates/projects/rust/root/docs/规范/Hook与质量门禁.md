# Hook 与质量门禁

这个文件描述当前仓库在提交阶段会机械执行什么。

## 执行入口

主要入口：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `tools/repo-check/src/main.rs`
- `repo-check.toml`

其中：

- `githooks/` 只负责把控制权交给 `repo-check`
- `tools/repo-check/` 承担 branch、commit、changelog 和 workspace gate
- `repo-check.toml` 记录 schema version 与当前仓库的 project kind / layout / manifest / changelog / primary source 路径，并作为这些路径检查的 source of truth

## `pre-commit` 当前做什么

按顺序，它会做这些事：

1. 校验分支名。
2. 如果没有 staged 文件，直接退出。
3. 对 manifest 的 major version change 做默认拒绝。
4. 校验 changelog 路径是否符合当前 layout。
5. 校验实际改动是否同步更新了正确的 changelog。
6. 拒绝 changelog-only commit。
7. 默认拒绝删除当前仍然活跃的 changelog。
8. 默认只允许修改 `[Unreleased]`。
9. 运行 Rust 项目的本地 gate。

补充说明：

- root layout 下，只有会触及发布面的实际改动才要求同步更新 `repo-check.toml` 配置的 changelog；纯文档和治理包装改动不会被强制写 changelog

## `commit-msg` 当前做什么

它会做三件事：

1. 再次校验分支名。
2. 强制 Conventional Commits。
3. 如果 manifest 发生非 `0` 大版本 major bump，要求提交消息显式声明 breaking change：
   - 标题带 `!`
   - 或 footer 使用 `BREAKING CHANGE:` / `BREAKING-CHANGE:`

## project gate

当前 `repo-check workspace local` / `ci` 的 Rust gate 是：

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo test --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `tools/repo-check` 保持独立 Rust workspace，通过 `cargo run --manifest-path tools/repo-check/Cargo.toml -- ...` 单独编译和执行，从而避免外层 Cargo workspace 污染模板或生成仓库

当前 `workspace local` 和 `workspace ci` 在 Rust 项目上执行同一套质量门禁。保留 `ci` 模式只是为了给 hook、本地手动检查和 CI 提供稳定、统一的调用入口。
为了避免检查器自己把仓库写脏，`workspace local` / `ci` 会先复制当前 worktree 快照，再在快照里执行 Rust gate；`pre-commit` 则对 staged snapshot 执行同样的检查。

## 两种运行形态

为了让检查器未来可以迁到其他目录，hook 当前支持两种运行形态：

1. repo-local manifest
2. external binary

repo-local manifest 默认是 `tools/repo-check/Cargo.toml`，也可以通过 `OMNE_REPO_CHECK_MANIFEST` 指向其他位置。

hook wrapper 会把 Unix 路径、Windows 盘符绝对路径和 UNC 路径都当作绝对路径处理，不会再错误拼到仓库根目录后面。

## 与远端分支保护的关系

本地 hook 和 `repo-check workspace local` 负责把高频问题尽量前移到开发机。

但它们不替代远端 gate：

- `main` 仍应配置为受保护分支
- 必需的 CI / CD status checks 应在合并前全部通过
- 远端 required checks 应与默认分支实际使用的 CI 入口保持一致
