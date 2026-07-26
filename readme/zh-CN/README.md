<div align="center">
  <h1>Cleanr</h1>
  <p><strong>让你的 AI 借助 Cleanr，安全地清理磁盘。</strong></p>
  <p>
    <a href="https://drl990114.github.io/cleanr/zh-Hans/">完整文档</a>
    ·
    <a href="https://github.com/drl990114/cleanr/releases">下载</a>
    ·
    <a href="https://github.com/drl990114/cleanr/discussions">讨论区</a>
  </p>
  <p>
    <a href="https://github.com/drl990114/cleanr/actions/workflows/ci.yml"><img alt="CI 工作流" src="https://img.shields.io/github/actions/workflow/status/drl990114/cleanr/ci.yml?branch=main&label=CI&style=flat-square&logo=githubactions&logoColor=white"></a>
    <a href="https://github.com/drl990114/cleanr/actions/workflows/release.yml"><img alt="发布工作流" src="https://img.shields.io/github/actions/workflow/status/drl990114/cleanr/release.yml?label=release&style=flat-square&logo=githubactions&logoColor=white"></a>
    <a href="https://github.com/drl990114/cleanr/blob/main/LICENSE"><img alt="MIT License" src="https://img.shields.io/github/license/drl990114/cleanr?style=flat-square&color=0f766e"></a>
    <a href="https://www.npmjs.com/package/cleanr-cli"><img alt="npm 版本" src="https://img.shields.io/npm/v/cleanr-cli?style=flat-square&logo=npm"></a>
  </p>
  <p>
    <img alt="Rust" src="https://img.shields.io/badge/Rust-1.94-000000?style=flat-square&logo=rust&logoColor=white">
    <img alt="Ratatui" src="https://img.shields.io/badge/Ratatui-0.29-2563eb?style=flat-square">
    <img alt="支持 macOS、Linux 和 Windows" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-475569?style=flat-square">
    <img alt="开源项目" src="https://img.shields.io/badge/open%20source-MIT-155eef?style=flat-square">
  </p>
  <p>
    <a href="../en/README.md">English</a>
    ·
    <a href="../../README.md">仓库 README</a>
    ·
    <a href="../../CONTRIBUTING.md">贡献指南</a>
  </p>
</div>

Cleanr 帮助开发者以及 macOS、Windows 用户发现可重建的生成文件与缓存，避免把磁盘清理变成盲删。它会扫描你选择的路径，说明每个候选项的匹配原因，让你在键盘驱动的终端界面里审阅清理计划，并把选中的项目移动到系统废纸篓。

## 为 AI 而设计

Cleanr 通过 `cleanr analyze` 向本地编码 Agent 提供确定、带版本的 JSON 证据，同时把
清理权限留给用户。Agent 无需解析终端输出，就能检查推荐状态、决策代码、风险提示和
扫描完整性。用户审阅并明确授权某个确切计划后，Agent 可以通过绑定摘要的命令，把
校验通过的条目移动到系统回收站，并保留本地恢复清单。除非你主动选择分享，否则原始
路径和报告始终留在本机。

直接从 GitHub 安装跨 Agent Skill `cleanr-review-disk-cleanup`：

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

Skill 会检查 Cleanr CLI 是否可用，在缺失时全局安装 `cleanr-cli`，并指导 Agent 完成
本地分析和经过明确授权的可恢复清理。支持的 Agent、报告契约和隐私说明请见
[证据与隐私](../../docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/evidence-and-privacy.md)。

## 特性

- 键盘驱动的扫描、审阅、清理和恢复流程。
- 内置规则覆盖常见开发者缓存、浏览器缓存、应用缓存、构建产物、包管理器缓存、大文件下载、日志和临时文件。macOS 还覆盖 Brave 和 Arc、常用桌面应用中明确命名的缓存目录、Homebrew、Xcode、CocoaPods、SwiftPM、诊断报告和下载的安装包。Windows 的保守覆盖只增加当前用户 Temp 和 DirectX 着色器缓存目录中长期未修改的普通文件。
- 每个候选项都会展示大小、置信度、匹配原因和风险提示。
- 提供仅限本机的 `cleanr analyze` JSON 契约，以及用于用户明确授权确切计划、绑定摘要的 `cleanr clean` 命令。
- 保守的默认选择策略：只有来自内置规则或可信规则的高置信度项目才可能被预选。
- 通过系统废纸篓清理、执行前再次校验、父子候选项去重和本地清理清单降低风险。
- 支持 macOS 废纸篓、Windows 回收站和兼容 Freedesktop 的 Linux 废纸篓恢复历史。
- 支持声明式插件，用于扩展清理规则和翻译。
- 提供 macOS、Linux 和 Windows 原生包，可通过 npm、Cargo 或 GitHub Release 安装。
- 支持英文和简体中文界面。

## 安装

通过 npm 安装：

```bash
npm install --global cleanr-cli
```

通过 Cargo 安装：

```bash
cargo install cleanr-cli
```

也可以从 [GitHub Releases](https://github.com/drl990114/cleanr/releases) 下载预编译二进制文件。

## 开始使用

在需要检查的目录中运行：

```bash
cleanr
```

或者指定一个或多个扫描根目录：

```bash
cleanr ~/projects ~/Downloads
```

进入 TUI 后，按 `s` 扫描，按 `r` 审阅候选项，按 `space` 选择或取消选择，按 `c` 确认清理。使用 `/scan --global` 可以检查已知系统清理位置；平台支持时，可使用 `/restore` 恢复历史清理运行。

在 TUI 中按 `?` 可查看快捷键帮助。

在 macOS 上，可以先只读检查常规的用户级位置：

```bash
cleanr analyze --global \
  --global-kind browser-caches \
  --global-kind app-caches \
  --global-kind logs \
  --global-kind temp-files \
  --global-kind downloads
```

需要同时检查包管理器和 Xcode 缓存时，再加入
`--global-kind developer-caches`。废纸篓内容、Mail 数据、iOS 备份、
Time Machine 快照、浏览器 Service Worker 和系统所有的根目录会被明确排除。

在 Windows 上，常规审阅范围会刻意收窄：

```bash
cleanr analyze --global \
  --global-kind app-caches \
  --global-kind temp-files
```

它只匹配当前用户 Temp 或 DirectX `D3DSCache` 目录中至少 30 天未修改的普通文件。
DirectX 着色器文件是 Windows 可以重新生成的图形缓存；Temp 与缓存目录本身永远
不会被选中。Explorer 缩略图数据库、崩溃转储、Windows Update 与传递优化数据、
Prefetch、回收站、注册表数据、Downloads 和系统所有的根目录会被明确排除。只有
用户明确把浏览器或开发者缓存纳入范围时，才应额外加入对应分类。

让本地编码 Agent 协助时，先使用只读分析；除非先主动脱敏，否则不要将 JSON 发送到
设备外：

```bash
cleanr analyze ~/projects > cleanr-analysis.json
```

报告只提供审阅证据，不是清理指令。如果用户希望委托执行，应先写入并审阅确切计划：

```bash
cleanr plan --output cleanr-plan.json ~/projects
cleanr clean --plan cleanr-plan.json \
  --plan-sha256 <reviewed-sha256> \
  --authorized-by-user
```

`plan` 和 `dry-run` 还支持可重复使用的
`--select <确切候选路径>` 与 `--deselect <确切候选路径>`。Agent 可以用它们记录
用户对需审阅候选项作出的明确选择，而无需编辑计划；未知、被重叠抑制或被安全策略
排除的路径会被拒绝。

`plan` 会打印文件的 SHA-256。`clean` 要求明确授权，会校验摘要、重新扫描并拒绝计划
漂移，然后把通过校验的条目移动到系统回收站并记录恢复清单，不会永久删除。

TUI、`analyze`、`plan` 和 `dry-run` 共用 `cleanr.toml` 中的
`[recommendations].preselect_after_days`（默认 90 天；设为 `0` 会关闭年龄门槛）。

## 安全模型

Cleanr 不会因为找到某个路径就直接清理。授权前你仍然可以编辑计划；选中路径会在
清理前再次校验；清理动作会移动到系统废纸篓，而不是永久删除。已授权计划发生变化
后，需要重新审阅并授权。

恢复能力依赖系统废纸篓，是尽力而为的机制。确认清理结果无误前，请不要清空系统废纸篓。

## 了解更多

- [快速开始](../../docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/quick-start.md)
- [使用 Cleanr](../../docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/using-cleanr.md)
- [安全与恢复](../../docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/safety-and-recovery.md)
- [配置](../../docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/configuration.md)
- [插件](../../docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/plugins.md)

## 参与贡献

开发环境、检查命令、文档工作流和发布说明请见 [CONTRIBUTING.md](../../CONTRIBUTING.md)。

## 致谢

Cleanr 的部分代码来源于
[Byron/dua-cli](https://github.com/Byron/dua-cli)。`dua-cli` 是由
Sebastian Thiel 及其贡献者以 MIT License 授权的磁盘使用分析工具。

## 许可证

Cleanr 使用 [MIT License](../../LICENSE) 授权。
