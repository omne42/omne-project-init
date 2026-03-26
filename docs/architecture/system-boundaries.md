# 系统边界

## 目标

`omne-project-init` 是一个仓库初始化器，不是通用模板引擎，也不是运行时治理框架。

它负责生成两类东西：

- 新项目的最小骨架
- 新项目的最小治理骨架

## 本仓负责什么

- 初始化 CLI
- project kind / layout 选择
- 模板文件复制与占位符替换
- `githooks/`
- `tools/repo-check/`
- agent-first 最小文档地图

## 不负责什么

- 项目运行时逻辑
- 通用模板 DSL
- 共享 `repo-check` 二进制分发
- 仓库外的治理服务

## 与其他仓库的关系

- `omne-project-init` 负责生成 `repo-check` 骨架
- 当治理规则稳定到足以跨仓共享时，应提升为独立 harness 项目
- 在此之前，本仓继续作为模板与引导入口存在
