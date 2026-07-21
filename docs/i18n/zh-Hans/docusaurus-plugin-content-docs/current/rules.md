---
sidebar_position: 6
description: 了解 Cleanr 为什么标记某个路径、置信度如何影响选择，以及内置规则覆盖什么。
---

# 规则与置信度

Cleanr 不会只看目录名就判断它可以移除。扫描条目会与带版本的**规则包**匹配，
规则会解释它是什么、为什么可以清理，以及重建它可能付出什么代价。

## 每个候选项包含什么

| 字段 | 含义 |
| --- | --- |
| 名称 | 便于理解的名称，例如“Rust target 目录” |
| 分类 | `build-cache`、`package-cache`、`downloads` 等分组 |
| 置信度 | `High`、`Medium` 或 `Low` |
| 原因 | 为什么该路径被视为清理候选项 |
| 风险说明 | 清理后可能出现的问题、耗时或网络下载 |
| 默认选择 | 规则是否请求预选该条目 |
| 匹配角色 | 具体规则使用 `primary`，宽泛证据规则使用 `fallback` |

同一个条目被多条规则匹配时，Cleanr 会保留全部命中作为证据。安全语义等价的规则
会以确定性方式解析；可信的具体 `primary` 规则可以在选择决策中遮蔽宽泛的
`fallback` 规则，但 fallback 仍会出现在报告和计划中。不可信规则不能遮蔽内置
fallback。其他语义分歧会保持为未解析冲突并要求人工审阅，而不是按分数静默选出
一条规则。最终计划还会移除互相重叠的父子候选项，避免重复计算空间。

## 置信度不是绝对保证

| 等级 | 建议 |
| --- | --- |
| `High` | 通常是生成或可下载数据；不熟悉的路径仍需审阅 |
| `Medium` | 往往可以重建，但代价可能较高，或包含仅存在于本机的状态 |
| `Low` | 可能是用户数据，必须谨慎人工确认 |

只有来自内置或可信来源、置信度为 `High` 且
`default_selected = true` 的规则才能预选条目。

批量选择只会改变 `Preselected` 和 `Available` 条目；包括未解析规则冲突在内的
`Review` 条目必须逐项选择。

## 内置规则包

### `builtin-dev`

内置插件 manifest `cleanr.builtin.dev` 提供 `builtin-dev` 规则包。除了已知的包管理器
和工具缓存，这个规则包还会通过项目感知规则识别生成的项目产物。这类规则先根据
marker 文件识别项目根，并可用项目根的直接子目录进一步约束，再只匹配相对于该项目
根声明的精确路径。仅凭目录名称，不足以把它判断为这类项目产物。

项目感知规则覆盖：

- Cargo、Node.js 和 React Native、Unity、Haskell、SBT、Maven、Gradle、CMake
  以及 Unreal Engine；
- Jupyter、Python、Pixi、Composer、Pub、Flutter、Elixir、Swift、Zig、Godot
  以及 .NET；
- Turborepo、Terraform 和 CocoaPods。

规则包仍然覆盖 Cargo registry 与 Git 依赖缓存、npm、pnpm、Yarn、pip、uv、Go
module、Xcode `DerivedData`、Next.js 和 Python 工具缓存等内容。Python `.venv`
目录被有意排除，因为其中可能包含重建成本很高、甚至无法精确重现的本地环境。其他
风险较高或可能包含本地状态的目录只供审阅，绝不会被预选；加入清理计划前，请阅读
对应的匹配原因和风险说明。

### `builtin-general`

查找需要人工审阅的通用候选项：

- Downloads 目录中至少 100 MiB 的文件；
- 至少 50 MiB 的 `.log` 文件；
- 至少 1 MiB 的 `.tmp` 文件。

这些规则有意设置为中低置信度，并且默认不选中。

### `builtin-system`

查找已知用户级系统清理候选项：

- 常见浏览器缓存目录；
- 应用缓存目录；
- 大型临时文件、日志和 Downloads 文件。

只有高置信浏览器缓存目录会被预选。应用缓存、临时文件、日志和 Downloads
默认只展示，需人工审阅。

## 启用或禁用规则包

只有 `cleanup.enabled_rule_packs` 中的 ID 会被加载：

```toml
[cleanup]
enabled_rule_packs = ["builtin-dev", "builtin-general", "builtin-system"]
```

如果只想关注开发者缓存，可以移除 `builtin-general` 和 `builtin-system`。

在 TUI 中运行 `/rules` 可以查看当前启用的规则包和规则。

## 添加自定义规则

推荐使用声明式插件 bundle。完整的最小示例、校验命令和信任模型见
[插件](./plugins)。

如果生成路径只有位于特定项目中才有意义，应使用 project matcher，而不是宽泛的
目录名或路径 glob。正向 marker 和根目录 glob 用来识别项目根，排除 glob 会否决
含糊的项目根，`artifact_paths` 则列出允许匹配的精确相对目录：

```toml
[rules.match]
kind = "directory"

[rules.match.project]
marker_globs = ["acme-project.toml"]
root_dir_globs = ["src"]
excluded_marker_globs = ["acme-keep-build"]
excluded_root_dir_globs = ["keep-output"]
artifact_paths = ["build/cache", "build/generated"]
```

这段配置应放在一个 `[[rules]]` 条目中。置信度、默认选择、匹配原因和风险说明仍应
保持保守，尤其是产物重建需要网络访问或可能包含仅存在于本机的状态时。排除 glob
只能否决同一次扫描快照中实际观察到的子项；被忽略的路径不能证明子项不存在，因此
绝不能把排除项作为规则唯一的安全边界。发布使用此 matcher 的 bundle 时，应把
`cleanr_version` 设为规则 schema 首次支持 `project` 的 Cleanr 版本；不要沿用最小
示例中通用的 `>=0.1.0` 下限。

插件目录中仍可以发现旧版独立 TOML 规则包，但 bundle 能提供版本和兼容性
元数据，因此更推荐使用。

### 路径 glob 与 fallback 语义

所有平台上的路径 glob 都按目录段匹配：`*` 只匹配一个路径段，绝不会跨越 `/`；
`**` 才能跨路径段递归匹配。例如，`**/Library/Caches/*` 会匹配 `Caches` 的直接
子项，但不会匹配其更深层后代。只有确实需要递归时才使用
`**/Library/Caches/**`。

只有刻意设计为宽泛兜底、并且仅在同一候选项没有可信 primary 规则时才应生效的
规则，才应设置 `match_role = "fallback"`。Fallback 规则不能设置
`default_selected = true`。只要能够表达明确的归属边界，就应优先使用具体 matcher
或 project matcher。
