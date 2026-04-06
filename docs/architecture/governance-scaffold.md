# 治理骨架契约

这个文件记录 `omne-project-init` 当前生成出的治理骨架有哪些稳定约束。

它只写仓库级事实，不展开 `repo-check` 的实现细节。

## 输出边界

当前生成的治理骨架至少包含：

- `githooks/pre-commit`
- `githooks/commit-msg`
- `repo-check.toml`
- `tools/repo-check/`
- `docs/规范/`
- `AGENTS.md`

其中：

- hook wrapper 只保留 runner 选择和参数转发
- 规则判断留在 Rust 的 `tools/repo-check`

## `--force` 重生成语义

`--force` 不是无条件覆盖。

当前只允许对这些目标目录执行安全重生成：

- 目录原本就是由 `omne-project-init` 生成
- 目标目录仍保留可解析的 `repo-check.toml`
- 目标目录的模板受管文件没有被手工改写
- 新模板不会覆盖旧 scaffold 之外的现存文件

通过这些检查后，初始化器才会先清理旧 scaffold 的受管文件，再写入新模板。

这条契约的目的不是保守，而是防止：

- 不同 project kind / layout 的旧生成物混在一起
- 手工文件被 `--force` 静默吃掉

## Rust workspace 归位

生成 Rust 项目时，`tools/repo-check` 会直接进入 workspace member。

这意味着生成仓库里的：

- `workspace local`
- `workspace ci`

都会把治理工具本身纳入 `fmt`、`check`、`test`、`clippy`。

## 配置路径契约

`repo-check.toml` 里的这些字段是有效配置，不是展示性元数据：

- `package_manifest_path`
- `changelog_path`

生成出的 `repo-check` 会按当前配置读取这些路径，而不是把根目录默认路径写死在逻辑里。

这条契约允许后续仓库在保持 hook 协议不变的前提下迁移：

- package manifest 位置
- changelog 位置
- `repo-check` manifest 位置

## crate layout 的 changelog 归属

crate layout 下，不只有 crate 自己的源码改动会触发 changelog 约束。

当前归属规则是：

- 真实 workspace member crate 的变更，更新各自 changelog
- 根级治理改动也必须落到主 crate 的 changelog
- 不在 workspace member 里的 `crates/*` 目录不会被误判成活跃 crate
- 删除一个 crate 时，允许连同它自己的 changelog 一起退场

这里的“根级治理改动”包括根级文档、hook、`repo-check.toml`、`.github/` 和 `tools/repo-check/` 这类仓库治理面。
