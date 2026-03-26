# Repo Map

这个仓库只做一件事：生成新的项目仓库。

它同时负责两部分输出：

- 项目本身骨架
- Rust 实现的仓库治理骨架

## 从哪里开始

- 想看 CLI 入口：`src/main.rs`
- 想看文档入口：`docs/README.md`
- 想看文档系统地图：`docs/docs-system-map.md`
- 想看仓库边界：`docs/architecture/system-boundaries.md`
- 想看源码与模板布局：`docs/architecture/source-layout.md`
- 想看模板目录：`templates/`
- 想看仓库目标与边界：`README.md`
- 想看生成出的治理工具：`templates/common/tools/repo-check/`
- 想看生成出的规范文档：`templates/common/docs/规范/`
- 想看自动化 smoke：`tests/cli_smoke.rs`
- 想看文档系统检查：`tests/docs_system.rs`

## 工作约束

- 模板应以文件形式保存在仓库里。
- 初始化器本体应使用 Rust。
- 生成出的 commit / pre-commit 检查逻辑应使用 Rust。
- 生成出的 Rust crate 使用 `edition = "2024"`。
- 生成出的 hook wrapper 只保留薄入口，核心规则放进 Rust 的 `repo-check`。
- 要兼顾 Windows、Linux、macOS。
- 新增项目类型时，优先补模板目录与 Rust 初始化逻辑，而不是重新塞回大段内嵌模板。
