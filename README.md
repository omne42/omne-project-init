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

CLI 会读取这些模板文件，替换占位符后写入目标仓库；如果项目模板与 `templates/common/` 渲染到同一路径，项目模板优先覆盖公共模板。Rust 相关模板统一使用 `edition = "2024"`。

## 生成结果

生成出来的仓库包含两层内容：

1. 项目本身的最小骨架
2. 仓库级治理骨架

仓库级治理骨架当前包含：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `repo-check.toml`
- `tools/repo-check/`
- `docs/README.md`
- `docs/docs-system-map.md`
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
- `docs/architecture/governance-scaffold.md`
- `docs/architecture/source-layout.md`

CI 与本地验证入口：

- `.github/workflows/ci.yml`
- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`

## CLI

当前入口：

```bash
cargo run -- init /path/to/new-repo --project rust --layout crate
cargo run -- init /path/to/new-repo --project rust --layout root
cargo run -- init /path/to/new-repo --project python
cargo run -- init /path/to/new-repo --project nodejs
cargo run -- manifest /path/to/new-repo --project rust --layout crate
```

`--force` 现在只用于安全重生成已经由 `omne-project-init` 生成过、且仍保留有效 `repo-check.toml` 的仓库。它会先校验旧 scaffold 的受管文件没有被手改，再清理旧 scaffold 的受管文件，然后写入新模板，避免不同 project/layout 的旧生成物被混在一起，也避免覆盖非生成文件。

生成 Rust 项目时，workspace 会把 `tools/repo-check` 直接纳入 member，确保 `workspace local` / `workspace ci` 的 `fmt`、`check`、`test`、`clippy` 会覆盖治理工具本身，而不是把治理工具排除在 workspace 之外。

生成 Python 项目时，模板会声明 `requires-python = ">=3.11"`；生成出的 `repo-check` 会按 `pyproject.toml` 当前声明选择兼容解释器，并在版本不满足时直接失败。

生成出的 `repo-check` 会按 `repo-check.toml` 当前配置读取 package manifest 和 changelog 路径；crate layout 下，主 crate 的 changelog 还负责承接根级治理改动。

生成后常用命令：

```bash
cargo run --manifest-path tools/repo-check/Cargo.toml -- install-hooks
cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace local
cargo run --manifest-path tools/repo-check/Cargo.toml -- workspace ci
```

## 仓库验证

当前仓库自己的最小验证入口分两层：

- 本地快速回归：`cargo test`
- 与 CI 对齐的警戒线：`cargo clippy --all-targets --all-features -- -D warnings`

GitHub Actions 定义位于 `.github/workflows/ci.yml`，当前至少覆盖这两条命令。

对生成出的仓库，远端 `main` 分支保护还应把 PR 实际会跑到的必需 job contexts 全部设为 required checks。只写“开启 CI”还不够；如果某个 workflow 在 PR 上会拆成多个 job，这些真正承接合并门禁的 job 名也应进入 required checks。

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
