# 文档系统地图

## 入口分工

- `README.md`
  - 对外概览、支持矩阵、本地验证命令和 CI 入口。
- `AGENTS.md`
  - 给执行者的短地图。
- `docs/`
  - 版本化事实来源。
- `.github/workflows/ci.yml`
  - 当前仓库的最小 GitHub Actions gate。

## 目录职责

- `docs/architecture/`
  - `system-boundaries.md`：初始化器本体、模板与治理骨架的边界。
  - `source-layout.md`：源码树与模板目录职责。

## 新鲜度规则

- 新增 project kind、layout 或治理输出时，更新 `system-boundaries.md`。
- 新增目录或模板归属变化时，更新 `source-layout.md`。
- 调整当前仓库的验证命令或 CI gate 时，更新 `README.md` 与 `.github/workflows/ci.yml` 的对应入口说明。
- 不把模板语义和 repo-check 归位规则留在聊天记录里。
- `tests/docs_system.rs` 机械检查 README / AGENTS / docs 入口是否仍然对齐。
