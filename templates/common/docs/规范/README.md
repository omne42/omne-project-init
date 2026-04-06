# 规范目录

当前仓库的提交阶段规范分成三类：

- `Hook与质量门禁.md`
- `提交与分支.md`
- `变更记录.md`

阅读顺序建议：

1. 先看 `../docs-system-map.md` 确认入口分工。
2. 再看这里的规范索引。
3. 最后进入对应主题文件。

它们对应的机械执行入口是：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `tools/repo-check/src/main.rs`

这里要把两件事分开：

- 当前仓库到底采用什么 layout
- 检查引擎本身支持哪些可迁移能力

当前仓库 layout 已固定为 `__LAYOUT_LABEL__`，但 `repo-check` 仍保留 `root` / `crate` 两种语义，方便未来迁移与复用。
