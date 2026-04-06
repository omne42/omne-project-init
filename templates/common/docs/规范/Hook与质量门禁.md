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
- `repo-check.toml` 记录当前仓库的 project kind / layout / manifest / changelog 路径，并作为这些路径检查的 source of truth

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
9. 运行 project-kind 对应的本地 gate。

补充说明：

- root layout 下，只有会触及发布面的实际改动才要求同步更新 `repo-check.toml` 配置的 changelog；纯文档和治理包装改动不会被强制写 changelog
- crate layout 下，root 级治理代码、workspace 根文档和其他不属于活跃 crate 的改动，会归属到 `repo-check.toml` 配置的主 changelog 路径
- `crates/<name>/` 下只有真实 crate 才会被当作 crate-package；共享目录不会自动变成 changelog owner
- 如果一个 crate 被整体删除，对应的 crate changelog 可以随之删除
- Rust crate layout 下，如果 workspace 根版本变更导致继承 `version.workspace = true` 的 crate 有效版本变化，相关 crate 的 changelog 也必须同步更新

## `commit-msg` 当前做什么

它会做三件事：

1. 再次校验分支名。
2. 强制 Conventional Commits。
3. 如果 manifest 发生非 `0` 大版本 major bump，要求提交消息带 `!`。

## project gate

当前 `repo-check workspace local` / `ci` 的 project gate 是：

### rust

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo test --workspace --all-features`
- `ci` 额外执行 `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Rust 模板生成的 workspace 会把 `tools/repo-check` 纳入 member，所以这些 gate 会直接覆盖治理工具本身

### python

- 先读取 `pyproject.toml` 的 `[project].requires-python`，并要求它继续守住模板契约 `>=3.11`
- 再选择满足该声明的解释器
- `python -m compileall __PY_PACKAGE__ tests`
- `python -m unittest discover -s tests -p 'test*.py'`

Windows 下如果 `python` 不可用，检查器会尝试 `python3`，再尝试 `py -3`；如果找到了 Python，但版本不满足 `requires-python`，gate 会直接失败。

### nodejs

- `node --check src/index.js`
- `node --test`

## 两种运行形态

为了让检查器未来可以迁到其他目录，hook 当前支持两种运行形态：

1. repo-local manifest
2. external binary

repo-local manifest 默认是 `tools/repo-check/Cargo.toml`，也可以通过 `OMNE_REPO_CHECK_MANIFEST` 指向其他位置。

hook wrapper 会把 Unix 路径、Windows 盘符绝对路径和 UNC 路径都当作绝对路径处理，不会再错误拼到仓库根目录后面。
