---
sidebar_position: 2
description: 安装 Cleanr，并安全完成第一次扫描、审阅和清理。
---

# 快速开始

## 1. 安装 Cleanr

选择一种安装方式即可。

### npm

需要 Node.js 18 或更高版本：

```bash
npm install --global cleanr-cli
```

npm 包会安装一个轻量启动器，以及与你的操作系统和 CPU 匹配的原生二进制。

### Cargo

需要 Rust 1.94 或更高版本：

```bash
cargo install cleanr-cli
```

### 预编译二进制

从 [GitHub Releases](https://github.com/drl990114/cleanr/releases)
下载适合当前平台的文件。在 macOS 或 Linux 上，需要赋予执行权限，并将文件
放到 `PATH` 中的目录。

### 从源码构建

```bash
git clone https://github.com/drl990114/cleanr.git
cd cleanr
cargo build --release
```

构建结果位于 `target/release/cleanr`。

## 2. 启动 TUI

在想要检查的目录中运行：

```bash
cleanr
```

也可以在启动时指定一个或多个扫描根目录：

```bash
cleanr ~/projects ~/Downloads
```

如需只为本次运行修改候选门槛而不改配置，可加入 `--inactive-days <天数>`：

```bash
cleanr --inactive-days 30 ~/projects
```

Cleanr 会先打开首页，**不会**在启动时自动扫描或清理任何内容。

## 3. 完成第一次清理

进入 TUI 后：

1. 按 `s` 扫描当前根目录。
2. 扫描完成后按 `r` 审阅达到当前观测修改时间年龄门槛的候选项。
3. 使用 `j`/`k` 或方向键移动。
4. 按 `space` 选择或取消选择。
5. 按 `c` 检查已选数量和大小，并进入确认。
6. 选择“是”，再按 `Enter` 将已选项移动到回收站。

随时按 `?` 查看快捷键。扫描过程中按 `Esc` 或 `x` 可以取消。

:::tip 第一次建议保守一些

先扫描单个项目，不要一开始就扫描整个主目录。确认清理前，逐项阅读匹配原因和
风险说明。

:::

## 扫描已知系统清理位置

按 `/` 打开命令面板，输入以下命令并按 `Enter`：

```text
/scan --global
```

它会扫描当前平台上已知的用户级清理位置，包括开发缓存、浏览器缓存、应用缓存、临时文件、日志和下载目录，并不代表“扫描整个磁盘”。

如需缩小全局扫描范围，可以添加一个或多个分类：

```text
/scan --global-kind browser-caches --global-kind logs
```

## 向本地 AI 工具提供只读证据

希望让其他本地 Agent 检查 Cleanr 的确定性事实，而不是驱动 TUI 时，可以使用
`analyze`：

```bash
cleanr analyze ~/projects > cleanr-analysis.json
```

该命令只扫描并输出带版本的 JSON 报告，会保留完整候选证据，包括未达到年龄门槛的
条目；不会创建清理计划，也不会移动文件。输出包含真实本地路径，除非已经独立去除
敏感信息，否则应保留在本机。契约和边界详见[证据与隐私](./evidence-and-privacy)。
长期门槛由 `[recommendations].preselect_after_days` 设置，默认 90 天；本次运行可用
`--inactive-days <天数>` 覆盖。修改时间只是文件系统元数据，并不能证明最后访问时间。

可直接从 GitHub 安装仓库中的跨 Agent Skill：

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

它默认保持只读。只有当前用户审阅计划摘要并明确授权其 SHA-256 后，Skill 才能执行
该确切计划；成功条目进入系统回收站，并保留恢复清单。Agent 选择和调用方式请见
[证据与隐私](./evidence-and-privacy)。

## 使用简体中文

初始化内置中文语言文件：

```bash
cleanr init --locale zh-CN
```

之后也可以在 TUI 中打开 `/languages`，选择语言并按 `Enter`。选择结果会
保存到默认配置文件。

## 接下来

- 在[使用 Cleanr](./using-cleanr)中查看全部快捷键和斜杠命令。
- 在[安全与恢复](./safety-and-recovery)中了解可恢复范围。
- 在[配置](./configuration)中排除目录或调整主题。
