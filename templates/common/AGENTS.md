# AGENTS

这个仓库按 agent-first 方式组织，但入口文档保持短小。

先看这些文件：

- `README.md`
- `docs/README.md`
- `docs/规范/README.md`
- `repo-check.toml`

如果你要改提交流程、hook 或本地门禁，再看这些路径：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `tools/repo-check/src/main.rs`

如果你要改项目本身，优先从这些位置建立上下文：

- 主代码入口：`__PRIMARY_SOURCE_PATH__`
- 主要验证命令：`__PRIMARY_VALIDATION_COMMAND__`
- 包清单：`__PACKAGE_MANIFEST_PATH__`
- changelog：`__CHANGELOG_PATH__`

当前仓库由 `omne-project-init` 生成，固定元信息如下：

- project kind: `__PROJECT_KIND__`
- layout: `__LAYOUT__`
- package name: `__PACKAGE_NAME__`

工作约束：

- 不要把 `AGENTS.md` 扩成百科全书；更细的事实放进 `docs/`。
- `githooks/` 只保留薄包装；真正检查逻辑放进 Rust 的 `tools/repo-check/`。
- 规则如果要长期生效，优先写成可执行检查，而不是只写口头约定。
- Windows、Linux、macOS 都是目标环境，避免引入 bash-only 的核心逻辑。
