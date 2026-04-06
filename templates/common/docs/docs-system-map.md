# 文档系统地图

## 入口分工

- `README.md`
  - 仓库概览、常用验证命令、治理骨架入口。
- `AGENTS.md`
  - 给执行者的短地图。
- `docs/README.md`
  - 文档目录入口。
- `docs/规范/`
  - 提交、分支、changelog、hook 和质量门禁规则。
- `repo-check.toml`
  - 当前仓库治理配置的 source of truth。

## 目录职责

- `docs/规范/README.md`
  - 规范目录索引。
- `docs/规范/提交与分支.md`
  - 分支命名、提交消息和主分支保护约束。
- `docs/规范/变更记录.md`
  - changelog 归属与更新规则。
- `docs/规范/Hook与质量门禁.md`
  - hook、本地 gate 与 CI 对齐关系。

## 新鲜度规则

- 调整入口文档分工时，同时更新 `README.md`、`AGENTS.md`、`docs/README.md` 和这里。
- 调整提交流程、hook、本地 gate 或 CI 对齐方式时，更新 `docs/规范/Hook与质量门禁.md`。
- 调整分支策略、合并策略或主分支保护要求时，更新 `docs/规范/提交与分支.md`。
- 不把长期治理规则只留在聊天记录里；规则需要进入仓库里的文档或可执行检查。
