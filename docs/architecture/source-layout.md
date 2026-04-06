# 源码布局

## 入口文件

- `src/main.rs`
  - CLI 入口与初始化流程。

## 模板目录

- `templates/common/`
  - 各 project kind 共用的治理骨架与文档骨架。
- `templates/projects/rust/`
  - Rust 项目模板。
- `templates/projects/python/`
  - Python 项目模板。
- `templates/projects/nodejs/`
  - Node.js 项目模板。

## 测试目录

- `tests/cli_smoke.rs`
  - 初始化器与生成物 smoke 测试。
- `tests/docs_system.rs`
  - 文档入口和文档地图检查。

## 布局约束

- 模板必须以真实文件形式保存在仓库中。
- 新 project kind 优先新增模板目录和最小 Rust 接线，不引入新的模板语言层。
- 治理骨架的稳定归位规则记录在 `docs/architecture/governance-scaffold.md`。
