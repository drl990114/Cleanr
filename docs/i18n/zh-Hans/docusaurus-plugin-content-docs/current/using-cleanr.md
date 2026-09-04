---
sidebar_position: 3
description: 了解 Cleanr 的扫描、审阅、清理、恢复、快捷键和斜杠命令。
---

# 使用 Cleanr

## 选择扫描范围

启动时传入的路径会成为默认扫描根目录：

```bash
cleanr ~/projects/app-one ~/projects/app-two
```

使用 `--inactive-days <天数>` 可以只覆盖本次运行的候选年龄，不修改配置文件：

```bash
cleanr --inactive-days 30 ~/projects/app-one
```

不传路径时使用当前目录。启动 Cleanr 不会立即扫描；需要按 `s` 或运行
`/scan`。

也可以在命令面板中替换当前扫描根目录：

```text
/scan /home/me/projects/app-one /home/me/Downloads
```

加上 `--global` 可以同时包含已知系统清理位置：

```text
/scan /home/me/projects --global
```

在命令面板中，按 `/`，输入 `global`，再按 `Enter`，即可选择
`/scan --global` 快捷项，不需要记住参数。

使用 `--global-kind` 可以缩小全局预设范围。传入分类时会自动启用全局扫描：

```text
/scan --global-kind browser-caches
```

只为一次扫描覆盖配置的修改时间年龄门槛：

```text
/scan --inactive-days 30
```

Windows 常规审阅建议使用更窄、仅包含普通文件的范围：

```text
/scan --global-kind app-caches --global-kind temp-files
```

在 Windows 上，这个范围只发现当前用户的 DirectX `D3DSCache` 和用户 Temp 目录。
对应的高置信度 Windows 规则只匹配至少 30 天未修改的普通文件，绝不匹配这两个目录
本身。只有用户希望审阅浏览器或开发者缓存时，才应额外加入 `browser-caches` 或
`developer-caches`。

TUI 中输入的路径不会经过 Shell 展开，因此 `~` 和环境变量会被当成普通文字。
请使用绝对路径。路径包含空格时，建议在启动 Cleanr 时通过带引号的参数传入。

## 审阅和选择候选项

扫描完成后按 `r` 或运行 `/review`。每行候选项显示大小、分类和路径；详情会显示
完整分类、命中规则、置信度、匹配原因和风险说明。默认只包含候选目录树中最新观测
修改时间达到配置门槛的条目；
门槛默认是 90 天。

来自内置规则或可信插件的高置信度条目可能会被预选。中低置信度条目，以及
未信任插件的所有匹配，默认不会选中。

长期门槛通过 `[recommendations].preselect_after_days` 修改；单次运行可使用
`--inactive-days <天数>` 覆盖。设为 `0` 会移除年龄过滤，显示其他方面仍符合条件的
全部候选项。修改时间只是文件系统元数据，并不能证明最后访问时间。

分类描述规则对应的内容，例如构建缓存、日志，与 `--global-kind` 选择的扫描位置
不同。内置分类使用翻译后的名称，插件自定义分类保留原名。有效规则存在跨分类冲突时，
候选项归入“多分类”，详情列出全部有效分类和冲突信息。

按 `f` 打开单分类筛选弹层，查看各分类的候选数量和大小。使用 `↑` / `↓` 或
`j` / `k` 选择，按 `Enter` 应用，按 `Esc` 取消。切换分类会保留跨分类勾选；列表
显示当前筛选数量和全局已选汇总，并提示筛选外已选数量与大小。切换页面保留筛选；
开始新扫描（包括清理后的自动重扫）会重置为“全部”。没有清理计划的部分结果仅显示
暂定分类，保持只读。

审阅时常用快捷键：

| 按键 | 作用 |
| --- | --- |
| `j` / `k`、`↓` / `↑` | 在列表中移动 |
| `gg` / `G` | 跳到第一项 / 最后一项 |
| `Ctrl+f` / `Ctrl+b` | 向下 / 向上翻页 |
| `space` 或 `Enter` | 选择或取消当前条目 |
| `f` | 打开分类筛选 |
| `a` 或 `%` | 全选当前筛选范围的所有页；已全部选中时取消该范围选择 |
| `Shift+A` | 全选全局候选项；已全部选中时取消全局选择 |
| `c` | 确认清理全部已选项，包含筛选外已选项 |
| `h` 或 `Esc` | 返回首页 |
| `?` | 打开快捷键帮助 |
| `q` | 退出 |

列表移动支持数字前缀，例如 `5j` 向下移动 5 项，`12G` 跳到第 12 项。

## 清理已选条目

按 `c` 或运行 `/clean`，检查已选数量和大小。默认配置下，Cleanr 会要求
确认，并且初始选中“否”。清理使用全局选择；分类筛选隐藏了已选项时，确认框
还会提示这些筛选外条目的数量和大小。

确认后，每个条目都会再次校验，然后移动到系统回收站。失败会逐项记录；
某一项失败不会掩盖其他条目的执行结果。

`/clean --confirm` 会跳过确认对话框，把当前选择作为本地用户的显式操作直接
执行。只应在已经审阅计划后使用。

## 恢复一次清理

运行 `/restore`，选择一条清理记录并按 `Enter`，确认后会尝试把可用条目移回
原路径。

以下情况可能导致恢复失败：

- 条目已经不在系统回收站；
- 原路径已经存在新的文件或目录；
- 操作系统无法识别原来的回收站条目；
- 当前平台不支持程序化恢复。

Cleanr 不会覆盖已经存在的恢复目标。

## 非交互命令

不需要打开 TUI 时，可以在脚本或终端中使用这些命令：

```bash
cleanr scan --json /path/to/project
cleanr analyze /path/to/project
cleanr plan --output cleanr-plan.json /path/to/project
cleanr --inactive-days 30 plan --output cleanr-plan.json /path/to/project
cleanr plan --output cleanr-plan.json --select /exact/candidate /path/to/project
cleanr dry-run --json /path/to/project
cleanr clean --plan cleanr-plan.json --plan-sha256 <reviewed-sha256> --authorized-by-user
cleanr restore list
cleanr restore run <run-id> --confirm
```

`analyze` 始终输出带版本、仅限本地的 `AnalysisReport` JSON，并保留完整候选证据，
包括未达到年龄门槛的条目；它不会创建清理计划或移动文件。输出包含真实本地路径，
除非自行完成脱敏，否则只应交给本地 Agent。`dry-run` 和 `plan` 只生成清理计划。

人类可读的 `cleanr scan` 候选数量会应用当前年龄门槛；`cleanr scan --json` 仍保留
原始扫描条目。

`plan` 和 `dry-run` 通常只保留满足当前修改时间年龄门槛的候选项。可以重复使用
`--select <路径>` 或 `--deselect <路径>`，记录证据审阅中对确切候选路径作出的选择。
显式 `--select` 可以纳入其他方面仍可选择、但修改时间较新或缺失的需审阅候选项。
目标路径必须存在、属于本次扫描的候选项，并且没有被重叠处理抑制或被安全策略排除。
Agent 只有在当前用户对该确切候选路径明确作出决定后，才能选择需审阅候选项。不要
编辑生成的计划文件。

`plan` 写入文件时会打印该文件的 SHA-256。`clean` 只用于当前用户已经审阅并明确
授权的确切计划。它会校验传入的摘要，重新扫描计划根目录，重新生成确定性计划，并在
已选目标、扫描来源或安全策略发生变化时拒绝执行。重新生成时会保留已审阅的确切
选择；仅限未选候选项的变化不会使这些动作失效。它只会把通过校验的条目移动到系统
回收站并记录执行清单，不会永久删除。恢复仍然要求显式传入 `--confirm`。

## 斜杠命令

按 `/` 打开命令面板。需要扫描结果的命令会在扫描完成后出现。

| 命令 | 作用 |
| --- | --- |
| `/scan [path...] [--global] [--global-kind=<kind>] [--inactive-days=<天数>]` | 扫描路径或已知系统清理位置，可覆盖本次扫描的年龄门槛 |
| `/scan --global` | 扫描所有已知系统清理位置 |
| `/usage [path...] [--global] [--global-kind=<kind>] [--inactive-days=<天数>]` | 扫描并打开磁盘用量摘要，可覆盖本次扫描的推荐摘要年龄门槛 |
| `/usage --global` | 扫描已知系统清理位置并打开用量摘要 |
| `/review` | 生成并显示清理候选项 |
| `/plan` | 生成当前清理计划 |
| `/clean` | 检查当前选择并请求确认 |
| `/clean --confirm` | 不显示对话框，直接执行当前选择 |
| `/export-plan [path]` | 导出 JSON 计划，默认文件为 `cleanr-plan.json` |
| `/restore` | 打开清理历史并恢复一次运行 |
| `/rules` | 查看启用的规则包和规则 |
| `/plugins` | 查看已加载的声明式插件 |
| `/languages` | 查看并切换已安装语言 |
| `/tasks` | 查看当前会话的任务活动 |
| `/help` | 打开快捷键帮助 |
| `/quit` | 退出 Cleanr |

`/stats` 是 `/usage` 的别名，`/lang` 是 `/languages` 的别名，`/q` 是
`/quit` 的别名。

## 只查看磁盘用量

按 `u` 或运行 `/usage`。它会执行扫描并打开以大小为主的视图，不会移动文件，
也不会自动执行清理计划。该视图保留完整用量条目；候选和已选摘要指标会应用当前
年龄门槛，`/usage --inactive-days <天数>` 可为本次扫描覆盖该门槛。

## 安全取消或退出

- 扫描过程中按 `Esc` 或 `x` 请求取消。
- 非扫描状态下，`Esc` 或 `h` 返回首页。
- `q` 或 `Ctrl+C` 退出 Cleanr 并恢复终端状态。
