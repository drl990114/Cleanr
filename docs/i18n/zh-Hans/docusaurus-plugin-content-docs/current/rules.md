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
| 平台 | 可选的 `macos`、`windows` 和/或 `linux` 限制 |
| 来源 | 固定到 revision、用于转化或只读审计的开源项目 |

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
module、Xcode `DerivedData`、Next.js 和 Python 工具缓存等内容。在 macOS 上，还会
发现 Homebrew、CocoaPods、SwiftPM、Go build、Deno、Cypress、Composer、Bun、Pub、
CoreSimulator 和其他明确命名的 Xcode 缓存。DeviceSupport 与 XCTest devices 需要
人工审阅；Xcode archives 是低置信候选项，因为保留的构建和 dSYM 可能无法重建。

Python `.venv` 目录被有意排除，因为其中可能包含重建成本很高、甚至无法精确重现的
本地环境。其他风险较高或可能包含本地状态的目录只供审阅，绝不会被预选；加入清理
计划前，请阅读对应的匹配原因和风险说明。

### `builtin-general`

查找需要人工审阅的通用候选项：

- Downloads 目录中至少 100 MiB 的文件；
- 至少 50 MiB 的 `.log` 文件；
- 至少 1 MiB 的 `.tmp` 文件。

这些规则有意设置为中低置信度，并且默认不选中。

### `builtin-system`

查找已知用户级系统清理候选项：

- Chrome、Chromium、Edge、Firefox、Safari、Brave 和 Arc 的浏览器缓存目录；
- macOS 标准应用缓存根目录，以及位于 Application Support 或应用容器中、路径明确
  的常用桌面应用缓存；
- Quick Look 缩略图、Zoom 更新安装包、用户日志和诊断报告；
- 当前 Windows 用户 Temp 和 DirectX `D3DSCache` 目录中长期未修改的普通文件；
- 大型临时文件和 Downloads 文件，包括 `.dmg`、`.pkg`、`.mpkg` 和 `.iso`
  安装文件。

只有已知可重建缓存才可能被预选，并且仍需通过统一的年龄和证据门槛。宽泛的应用
缓存、Spotify 持久缓存、日志、诊断报告、通用临时文件匹配和 Downloads 都只供
审阅。选择某个应用的缓存前，应先退出该应用。

macOS 白名单参考了 [Dusty](https://github.com/yagcioglutoprak/dusty) 和
[PureMac](https://github.com/momenbasel/PureMac)，并按 Cleanr 的“废纸篓加恢复清单”
模型进一步收窄。Cleanr 明确排除废纸篓内容、Mail 数据、iOS 备份、Time Machine
快照、浏览器 Service Worker、Docker prune 动作和系统所有的根目录。

Windows 白名单刻意只包含普通文件。Windows 专属规则要求文件至少 30 天未修改才会
匹配：

- **用户临时文件**是当前用户 `AppData\Local\Temp` 下的普通文件；Temp 目录和
  子目录本身都不是候选项；
- **DirectX 着色器缓存文件**是 `AppData\Local\D3DSCache` 下由图形系统生成的
  普通缓存文件；Windows 可以按需重建，但图形应用下次启动时可能需要重新编译。

Cleanr 不会停止应用。如果 Windows 锁定了某个候选文件，移入回收站会失败，原文件
保持不变。Explorer 缩略图数据库需要成熟清理器重启 Explorer 才能释放，因此不会
纳入。用户级崩溃转储只以低置信度诊断项供人工审阅；系统所有的崩溃转储、Windows
Update 与传递优化数据、Prefetch、回收站、注册表数据和系统所有的根目录不会进入
清理计划。

Windows 路径参考了
[BleachBit](https://github.com/bleachbit/bleachbit/tree/ab0e4b94e29b8233adbe7ab010656e61b162c63d)
和
[Winapp2](https://github.com/MoscaDotTo/Winapp2/tree/3c0156de665cc180edc76745e425412ccc4356ca)，
再根据微软对
[存储感知临时文件清理](https://learn.microsoft.com/windows/client-management/mdm/policy-csp-storage#allowstoragesensetemporaryfilescleanup)
以及
[可重建 DirectX 与缩略图缓存](https://techcommunity.microsoft.com/blog/filecab/creating-remediation-actions-for-system-insights/428234)
的说明独立收窄。Cleanr 不会打包外部清理规则数据库或可执行文件。平台专属扫描根
只会在对应操作系统构建中注册，共享的 `builtin-system` 插件负责提供声明式解释。

新增覆盖还包括 Windows 浏览器与明确命名的 Electron 应用缓存、Linux 桌面缩略图和
部分 Flatpak 应用缓存，以及平台专属下载的安装包。Windows Update、macOS 软件更新
和 Linux 系统包缓存会报告为 `os_managed`，绝不会成为清理候选项或计划条目。

## 上游来源策略

内置规则会记录仓库、完整 commit、许可证与引用关系。宽松许可证来源可以在保留归属
后转化；GPL 和 ShareAlike 清理数据库只能标记为 `audited-against`：它们可以帮助发现
覆盖缺口，但 Cleanr 必须独立验证并重新编写规则，不复制或打包其数据库。运行
`node scripts/check-rule-sources.mjs` 可以校验固定的来源台账。

`platforms` 会阻止本来合法的路径模式在其他操作系统上误匹配；省略该字段时保持旧版
跨平台行为。

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
