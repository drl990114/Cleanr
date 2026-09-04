---
sidebar_position: 2
description: 安装 Cleanr，并完成一次目录或已知清理位置的只读审阅。
---

# 快速开始

从一个你熟悉的目录开始，例如下载目录或旧项目。第一次的目标是看到并理解候选项，无需执行清理。

## 1. 安装并确认版本

选择一种方式：

| 方式 | 要求 | 安装 |
| --- | --- | --- |
| npm | Node.js 18 或更新版本 | `npm install --global cleanr-cli` |
| Cargo | Rust 1.98 或更新版本 | `cargo install cleanr-cli` |
| 原生二进制 | 匹配系统与 CPU | 从 [GitHub Releases](https://github.com/drl990114/Cleanr/releases) 下载 |

然后运行：

```bash
cleanr --version
cleanr --help
```

手动下载时，根据机器选择 target：

| 平台 | Release target |
| --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux x86-64 | `x86_64-unknown-linux-musl` |
| Linux ARM64 | `aarch64-unknown-linux-musl` |
| Linux ARMv7 hard-float | `arm-unknown-linux-gnueabihf` |
| Windows x64 | `x86_64-pc-windows-msvc` |
| Windows x86 | `i686-pc-windows-msvc` |

macOS/Linux 可用 `uname -m` 查看架构；Windows 可看“设置 → 系统 → 系统信息 → 系统类型”。
当前列表没有原生 Windows ARM64 包。提供安装包与实际验证是两回事，详见[验证矩阵](./support-matrix.md)。

如果下载的是压缩包，先解压。macOS/Linux 使用 `chmod +x /path/to/cleanr` 使文件可执行，
再放入 `PATH` 中用户拥有的目录，例如 `~/.local/bin`；需要时把该目录加入 Shell 的
`PATH`。Windows 把 `cleanr.exe` 放入用户 `Path` 中的目录，再打开一个新终端。
PowerShell 可用 `.\cleanr.exe --version` 测试当前目录中的文件。

操作系统拦截下载文件时，先确认来源和 Release 资产。除非该版本明确说明，Cleanr
不会承诺公证或签名状态；不要关闭操作系统的安全保护。

## 2. 只审阅，不清理

传入真实目录，路径含空格时加引号：

```bash
cleanr "/path/to/folder"
```

例如，目录存在时，POSIX Shell 可使用 `cleanr "$HOME/projects/my-app"`，PowerShell
可使用 `cleanr "$HOME\projects\my-app"`。启动只设置扫描根目录，不会立即扫描。

1. 按 `s` 扫描目录。
2. 扫描完成后按 `r` 审阅候选项。
3. 用方向键或 `j` / `k` 移动，阅读原因与风险说明。
4. 按 `?` 查看帮助，或按 `q` 退出。

扫描完成并能理解结果，就算完成了第一次体验，即使没有候选项。扫描过程中可用
`Esc` 或 `x` 取消。

**为什么列表可能为空？** 审阅通常只显示匹配规则、且候选目录树中最新观测修改时间
至少达到 **90 天**的条目。最近修改的项目、排除路径、没有命中规则的目录都可能导致
空结果。扫描根目录本身不会成为清理候选项：请扫描包含 `target` 或 `node_modules`
的项目，而不是把这些生成目录自身作为根目录。这不表示整台电脑已经干净。

想查看完整规则证据而不移动文件，可以运行：

```bash
cleanr analyze "/path/to/folder"
```

`analyze` 保留未达到年龄门槛的候选项。确实希望更改本次审阅门槛时，可使用
`cleanr --inactive-days 30 "/path/to/project"`。`0` 移除年龄过滤，不会取消安全检查。
长期设置为 `[recommendations].preselect_after_days`。修改时间不能证明最后使用时间。

## 3. 选择后续操作

### 审阅一次清理

读过候选项后，按 `space` 调整选择、`c` 查看已选总量并打开确认框。默认配置下，
只有希望把这些条目移入回收站时，才选择“是”并按 `Enter`。

回收站通常仍占用文件对应的磁盘空间。候选大小和已移动字节数不是实测的可用空间。
`/restore` 打开清理历史；恢复需要回收站条目与清单，且不会覆盖原路径上已有的内容。
清理前请阅读[安全与恢复](./safety-and-recovery.md)。

### 配合 AI Agent 审阅 {/* #ai-agent */}

安装可选的跨 Agent Skill：

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

先让 Agent 解释所选清理范围内的候选项，例如应用缓存和临时文件，再由你选择操作。
Skill 会安装缺失的 CLI，但不会升级已有版本。`cleanr analyze` 是只读命令。Cleanr 不会上传报告，但 Agent 即使在
本机执行工具，也可能把输出发送给云端模型。请先阅读[证据与隐私](./evidence-and-privacy.md)。

需要保存报告时，把它放在扫描根目录之外。Shell 重定向会创建或截断输出文件，这与
Cleanr 的只读扫描是不同的写入操作。不要向 issue 或外部服务提交原始 JSON。

### 扫描已知清理位置

按 `/` 打开 TUI 命令面板，输入较小范围：

```text
/scan --global-kind app-caches --global-kind temp-files
```

`--global` 表示已知的用户级位置，不是整块磁盘。需要审阅时，再加入浏览器、日志、
开发缓存或下载分类。覆盖范围因平台而异。Windows 的 `app-caches` 包括已知应用
缓存目录；用户 Temp 和 DirectX `D3DSCache` 的两条通用规则只选择较旧的普通文件，
不会选择这两个目录本身。详见[使用 Cleanr](./using-cleanr.md)。

## 升级、回退与卸载

使用相同安装方式，避免出现多个可执行文件。macOS/Linux 用 `command -v cleanr`，
PowerShell 用 `Get-Command cleanr` 查看实际执行的版本路径。变更版本前先阅读发行说明。

| 方式 | 升级 | 卸载 |
| --- | --- | --- |
| npm | `npm install --global cleanr-cli@latest` | `npm uninstall --global cleanr-cli` |
| Cargo | `cargo install cleanr-cli --locked --force` | `cargo uninstall cleanr-cli` |
| 原生二进制 | 用对应平台的新 Release 资产替换可执行文件 | 只移除该可执行文件和自己添加的 PATH 配置 |

安装某个已经发布的版本时，将 `X.Y.Z` 替换为它的版本号，使用
`npm install --global cleanr-cli@X.Y.Z` 或
`cargo install cleanr-cli --version X.Y.Z --locked --force`。
手动安装则下载该版本对应资产。回退二进制不代表旧版本一定能读取新配置、计划或清单；
请查看[兼容性说明](./support-matrix.md)。

升级前，按需在本地保留配置及清理/恢复状态的副本。`cleanr config path` 显示默认配置
路径，`cleanr restore list` 列出历史运行。卸载程序不意味着删除这些记录或清空回收站；
仍需恢复时，请保留两者。

## 语言与帮助

`cleanr init --locale zh-CN` 可以初始化中文语言文件，也可在 `/languages` 选择
已安装的语言。初始化可能写入配置或语言文件，独立于前面的只读体验。

[使用 Cleanr](./using-cleanr.md)介绍快捷键，[故障排查](./troubleshooting.md)说明安装与空结果问题。
分类筛选、跨筛选累积选择和 `Shift+A` 全局选择需要 **0.15.0 或更新版本**。
该版本同时引入 `cleanr.restore.v2` 记录；切换版本前请阅读[兼容性说明](./support-matrix.md)。
