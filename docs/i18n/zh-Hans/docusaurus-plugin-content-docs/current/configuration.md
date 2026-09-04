---
sidebar_position: 5
description: 配置 Cleanr 的扫描忽略、清理确认、插件、语言和主题。
---

# 配置

Cleanr 使用严格校验的 TOML 配置。第一次运行通常不需要创建文件，因为程序内置
了合理的默认值。

## 查找或创建配置

打印当前平台的默认配置路径：

```bash
cleanr config path
```

创建默认文件，但不覆盖已有配置：

```bash
cleanr config init
```

只有确定要替换现有文件时才使用 `--force`：

```bash
cleanr config init --force
```

单次运行使用其他文件：

```bash
cleanr --config ./cleanr.toml ~/projects
```

## 默认配置

```toml
[scan]
stay_on_filesystem = false
ignore_dirs = []
ignore_patterns = ["**/.git", "**/.git/**"]
global_kinds = ["developer-caches", "browser-caches", "app-caches", "temp-files", "logs", "downloads"]

[cleanup]
default_action = "trash"
require_confirm = true
enabled_rule_packs = ["builtin-dev", "builtin-general", "builtin-system"]

[recommendations]
preselect_after_days = 90

[plugins]
# dirs 默认指向平台配置目录下的 cleanr/plugins
trusted = []

[i18n]
# locale 依次读取 LC_ALL、LC_MESSAGES、LANG，最后回退到 en-US
# locale = "zh-CN"
# dirs 默认指向平台配置目录下的 cleanr/languages

[ui]
# "auto" 检测终端背景，也可以显式使用 "dark" 或 "light"
theme = "auto"
```

## 从命令行修改常用配置

可以直接编辑 TOML，也可以使用点号键名：

```bash
cleanr config get ui.theme
cleanr config set ui.theme dark
cleanr config set scan.stay_on_filesystem true
cleanr config set scan.budgets.max_entries 1000000
cleanr config set scan.budgets.max_elapsed_seconds 180
cleanr config set scan.budgets.max_estimated_memory_mib 512
cleanr config set scan.budgets.max_issue_details 1024
cleanr config set cleanup.require_confirm false
cleanr config set recommendations.preselect_after_days 180
cleanr config set i18n.locale zh-CN
```

布尔值支持 `true`/`false`、`yes`/`no`、`on`/`off` 和 `1`/`0`。未知键或
无效值会被拒绝，不会替换原本有效的配置。

只覆盖本次运行、不写入配置时，可以使用：

```bash
cleanr --inactive-days 30 ~/projects
```

## 配置参考

### `[scan]`

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `stay_on_filesystem` | `false` | 为 `true` 时不跨文件系统边界 |
| `ignore_dirs` | `[]` | 需要跳过的精确目录路径 |
| `ignore_patterns` | Git 元数据 glob | 同时匹配绝对路径和根目录相对路径的 glob |
| `global_kinds` | 全部内置分类 | `/scan --global` 使用的系统清理分类 |

已知绝对目录适合放入 `ignore_dirs`，重复出现的目录名或布局适合使用
`ignore_patterns`：

```toml
[scan]
ignore_dirs = ["/home/me/projects/large-fixture"]
ignore_patterns = ["**/.git/**", "**/vendor/**", "**/.venv/**"]
```

#### 可选扫描预算

默认配置文件不写入预算，四项默认都是 `0`（无限制），因此不会改变原有扫描覆盖。
对于异常庞大的全局扫描，可以选择下面这组保守配置；它只是建议示例，并非默认值：

```toml
[scan.budgets]
max_entries = 1000000
max_elapsed_seconds = 180
max_estimated_memory_mib = 512
max_issue_details = 1024
```

`max_entries` 限制成功保留的 `ScanEntry` 数量及其 O(N) 后处理内存；若要限制遍历工作量，
应使用耗时预算。内存值是对保留条目、路径、诊断和聚合临时结构的保守分配估算，
不是进程 RSS。任一预算命中都会在仍含本地路径的只读部分证据旁记录一份不含路径的预算
账本，且不能生成或执行清理计划。启用任一预算时扫描使用单个遍历 worker，以便在记录
进入报告前执行上限；部分结果的具体子集不保证跨运行完全相同。耗时上限包含根路径规范
化，并会在发现、元数据读取和聚合边界检查，无法中断已经阻塞在操作系统内核中的文件系统
调用。

### `[cleanup]`

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `default_action` | `"trash"` | 清理动作，目前仅支持 `"trash"` |
| `require_confirm` | `true` | 本地用户直接清理前是否弹出确认 |
| `enabled_rule_packs` | 内置规则包 | 需要加载的规则包 ID |

关闭确认只会改变对话框，执行层仍要求本地用户操作。详见
[安全与恢复](./safety-and-recovery.md)。

### `[recommendations]`

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `preselect_after_days` | `90` | 用于普通候选集和确定性预选的观测修改时间年龄门槛；`0` 会移除年龄过滤，接受 `1` 到 `3650` 的值 |

普通 TUI 审阅、`cleanr plan` 和 `cleanr dry-run` 只保留其他方面仍符合条件、且候选
目录树中最新观测修改时间至少达到该门槛的条目。`--inactive-days <天数>` 只覆盖本次
运行，不会写入配置。设为 `0` 时会显示其他方面仍符合条件的全部候选项。

`cleanr analyze` 和 TUI `/usage` 会保留完整证据；`/usage` 的候选和已选摘要指标仍
使用当前门槛。显式 `--select` 可以把其他方面仍可选择、但修改时间较新或缺失的需
审阅候选项加入计划。修改时间只是观测到的文件系统元数据，并不能证明最后访问时间；
未来、部分或不完整的证据仍会阻止自动预选。

## 外部本地 AI 工具

Cleanr 不内置模型、Provider、endpoint 或 API Key 配置。同一台机器上的外部
Agent 可以读取只读的 `cleanr analyze` JSON 契约，但分析本身不会授予清理能力。
委托清理还需要单独审阅的计划、其 SHA-256，以及当前用户的明确授权。报告包含本次
生效的推荐策略快照和真实本地路径，不能作为安全的远程分享格式；交给其他工具前请先
阅读[证据与隐私](./evidence-and-privacy.md)。

### `[plugins]`

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `dirs` | 平台 Cleanr 插件目录 | 存放插件 bundle 或旧版规则文件的目录 |
| `trusted` | `[]` | 允许预选高置信度候选项的插件 ID |

信任第三方 bundle 前请先阅读[插件](./plugins.md)。

### `[i18n]`

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `locale` | 环境变量，最后为 `en-US` | 当前语言，例如 `en-US` 或 `zh-CN` |
| `dirs` | 平台 Cleanr 语言目录 | 存放语言 YAML 文件的目录 |

`cleanr init --locale zh-CN` 会安装内置语言文件并更新这些设置。

### `[ui]`

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `theme` | `"auto"` | `"auto"`、`"dark"` 或 `"light"` |

## 配置校验错误

Cleanr 会拒绝未知字段、不支持的枚举值、空 ID，以及重复的可信插件或规则包
ID。编辑后无法启动时，请使用相同的 `--config` 路径重新运行，并根据错误定位
字段；程序不会静默修复配置文件。

Agent 可以在本机执行工具，同时把输出发送给云端模型。授权参数是调用方声明，
不是独立的人类身份认证或操作系统沙箱。详见[证据与隐私](./evidence-and-privacy.md)。
