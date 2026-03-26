# omne-project-init

`omne-project-init` 是一个 Rust CLI，用来初始化新的项目仓库和最小仓库治理骨架。

当前实现已经覆盖两个层面：

- 初始化项目骨架
- 初始化 `githooks/`
- 初始化基于 Rust 的 `repo-check`
- 初始化 agent-first 的最小文档地图

## 当前范围

支持的 project kind：

- `rust`
- `python`
- `nodejs`

支持的 layout：

- `rust`
  - `root`
  - `crate`
- `python`
  - `root`
- `nodejs`
  - `root`

目标环境：

- Windows
- Linux
- macOS

约束：

- 初始化器本体使用 Rust
- 生成出的治理检查器也使用 Rust
- 模板以真实文件形式维护，不使用 `.tmpl`
- 不生成 `.omne-project-init.json`

## 模板组织

模板文件直接存放在仓库里，不使用 `.tmpl` 后缀。

当前模板目录：

- `templates/common/`
- `templates/projects/rust/`
- `templates/projects/python/`
- `templates/projects/nodejs/`

CLI 会读取这些模板文件，替换占位符后写入目标仓库。Rust 相关模板统一使用 `edition = "2024"`。

## 生成结果

生成出来的仓库包含两层内容：

1. 项目本身的最小骨架
2. 仓库级治理骨架

仓库级治理骨架当前包含：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `repo-check.toml`
- `tools/repo-check/`
- `docs/规范/`
- `AGENTS.md`

`tools/repo-check/` 是一个随仓库一起生成的 Rust 工具，用来承接：

- hook 安装
- branch name 检查
- Conventional Commits 检查
- manifest major bump 检查
- changelog 检查
- 本地 workspace / project gate

其中 hook runner 支持两种形态：

1. repo-local manifest
   - 默认 `tools/repo-check/Cargo.toml`
   - 或通过 `OMNE_REPO_CHECK_MANIFEST` 指向迁移后的新位置
2. external binary
   - `OMNE_REPO_CHECK_BIN`
   - 或 PATH 上的 `omne-repo-check` / `repo-check`

这让检查器未来迁目录时，不需要重写整套 hook 协议。

## 文档入口

这个仓库采用短入口 + 分层事实文档：

- `AGENTS.md`
- `docs/README.md`
- `docs/docs-system-map.md`
- `docs/architecture/system-boundaries.md`
- `docs/architecture/source-layout.md`

## CLI

当前入口：

```bash
cargo run -- init /path/to/new-repo --project rust --layout crate
cargo run -- init /path/to/new-repo --project rust --layout root
cargo run -- init /path/to/new-repo --project python
cargo run -- init /path/to/new-repo --project nodejs
cargo run -- manifest /path/to/new-repo --project rust --layout crate
```

生成后常用命令：

```bash
cargo run --manifest-path tools/repo-check/Cargo.toml -- install-hooks
cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local
cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace ci
```

## 已验证内容

当前已经有可直接执行的自动化 smoke validation：

```bash
cargo test
```

主要覆盖：

- `omne-project-init` 本体 `cargo check`
- `manifest` 对 `rust/python/nodejs` 三类模板输出
- 生成出的 `tools/repo-check` 在 `rust root`、`rust crate`、`python`、`nodejs` 项目中执行 `workspace local`
- 生成出的 `repo-check` 在 `rust crate` 仓库里执行 `install-hooks`、`pre-commit`、`commit-msg`

对应测试文件：

- `tests/cli_smoke.rs`
- `tests/docs_system.rs`
