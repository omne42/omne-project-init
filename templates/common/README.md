# __REPO_NAME__

这是一个由 `omne-project-init` 初始化的仓库。

当前仓库语义：

- project kind: `__PROJECT_KIND__`
- layout: `__LAYOUT_LABEL__`
- package manifest: `__PACKAGE_MANIFEST_PATH__`
- changelog path: `__CHANGELOG_PATH__`
- primary source: `__PRIMARY_SOURCE_PATH__`

## 快速开始

初始化 hooks：

```bash
cargo run --manifest-path tools/repo-check/Cargo.toml -- install-hooks
```

手动执行本地门禁：

```bash
cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local
```

对 Rust 仓库，这条命令会同时执行：

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo test --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`

CI 复用入口：

```bash
cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace ci
```

当前 `workspace ci` 会沿用同一套 project gate，保留这个入口主要是为了让本地、hook 和 CI 使用同一个命令面。

## 跨平台说明

仓库治理的核心逻辑由 Rust 的 `tools/repo-check/` 承担，目标环境是：

- Windows
- Linux
- macOS

`githooks/` 仍然保留最薄的一层 shell 包装，这是为了兼容 git hook 入口；真正的规则和检查逻辑不写在 shell 里。

当前 hook 运行器支持两种形态：

1. repo-local manifest：
   `cargo run --manifest-path tools/repo-check/Cargo.toml -- ...`
2. external binary：
   通过 `OMNE_REPO_CHECK_BIN` 或 PATH 上的 `omne-repo-check` / `repo-check`

如果未来把 check crate 迁到别的目录，可以设置：

```bash
OMNE_REPO_CHECK_MANIFEST=path/to/repo-check/Cargo.toml
```

这样不需要立刻重写 hook 逻辑。

## 文档地图

- `AGENTS.md`：agent 入口地图
- `docs/README.md`：文档目录
- `docs/docs-system-map.md`：文档系统地图
- `docs/规范/`：提交、changelog、hook 规则
- `repo-check.toml`：当前仓库检查器配置，也是 manifest / changelog / primary source 路径的 source of truth

## 远端门禁

本地 hook 和 `repo-check workspace local` 只负责把问题前移到开发机。

真正保护默认分支时，还应在远端把 PR 实际会运行的必需 CI/CD job contexts 全部设为 required checks。不要只要求 workflow 名；如果一个 workflow 会展开成多个 job，应把真正承接合并门禁的 job 名逐个纳入 required checks。
