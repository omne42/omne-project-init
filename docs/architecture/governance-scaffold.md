# 治理骨架语义

## 目标

这个文件记录 `omne-project-init` 当前生成出的仓库治理骨架有哪些稳定语义。

它只说明已经在模板、CLI 和 smoke 里被反复依赖的事实，不展开实现细节。

## 当前组成

生成出的治理骨架至少包含：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `repo-check.toml`
- `tools/repo-check/`
- `docs/规范/`
- `AGENTS.md`

其中：

- hook wrapper 只负责把控制权交给 `repo-check`
- 核心规则以 Rust 实现，位于 `tools/repo-check/`
- `repo-check.toml` 是生成仓库里的治理契约入口

## `repo-check.toml` 的稳定职责

`repo-check.toml` 当前至少固定这些字段：

- `project_kind`
- `layout`
- `package_manifest_path`
- `changelog_path`
- `crate_dir`

这些字段不是展示用元数据。

`repo-check` 会按它们来决定：

- 当前仓库形态是 `root` 还是 `crate`
- 版本门禁应该读取哪个 manifest
- changelog gate 应该认哪个 changelog 入口
- crate layout 下哪个包是 primary package

也就是说，生成仓库后如果治理文件迁目录，正确做法是同步更新 `repo-check.toml`，而不是假设工具永远只认硬编码路径。

## Rust workspace 语义

生成 Rust 仓库时，`tools/repo-check` 会直接成为 workspace member。

这条约束的目的是让生成仓库自己的治理工具也被同一套本地 gate 覆盖，包括：

- `workspace local`
- `workspace ci`

因此，生成 Rust 仓库时不再把 `tools/repo-check` 仅仅视作旁挂工具目录。

## crate layout 的 changelog 归属

当生成仓库采用 Rust `crate` layout 时：

- 每个活跃 workspace crate 维护自己的 changelog
- root 级治理改动不会落到一个独立“仓库 changelog”
- 这类 root / governance 变更由 primary package 的 changelog 负责承接

这里的 governance/root 变更包括仓库级入口、hook、治理工具和同类骨架内容。

这条语义的目的，是避免 crate layout 仓库出现“代码改了，但没有任何受管 changelog 负责它”的灰区。

## `--force` 的安全边界

`init --force` 当前不是“强制覆盖任意目录”的语义，而是“安全重生成已受管 scaffold”的语义。

只有在下面条件同时成立时，它才会先清理旧生成物再写入新模板：

- 目标目录里有有效的 `repo-check.toml`
- 该目录确实是之前由 `omne-project-init` 生成
- 旧 scaffold 的受管文件没有被手工修改
- 新 scaffold 不会覆盖非受管文件

如果这些条件不满足，CLI 会拒绝执行，而不是冒险把新旧模板混在一起。
