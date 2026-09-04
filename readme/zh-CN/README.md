<div align="center">
  <h1>Cleanr</h1>
  <p><strong>审阅开发者缓存，由你决定哪些进入回收站。</strong></p>
  <p>
    <a href="https://drl990114.github.io/Cleanr/zh-Hans/">完整文档</a> ·
    <a href="https://github.com/drl990114/Cleanr/releases">下载</a> ·
    <a href="https://github.com/drl990114/Cleanr/discussions">讨论区</a>
  </p>
  <p>
    <a href="https://github.com/drl990114/Cleanr/actions/workflows/ci.yml"><img alt="CI 工作流" src="https://img.shields.io/github/actions/workflow/status/drl990114/Cleanr/ci.yml?branch=main&label=CI&style=flat-square"></a>
    <a href="https://www.npmjs.com/package/cleanr-cli"><img alt="npm 版本" src="https://img.shields.io/npm/v/cleanr-cli?style=flat-square"></a>
    <a href="../../LICENSE"><img alt="MIT License" src="https://img.shields.io/github/license/drl990114/Cleanr?style=flat-square"></a>
  </p>
  <p><a href="../en/README.md">English</a> · <a href="../../README.md">仓库 README</a> · <a href="../../CONTRIBUTING.md">贡献指南</a></p>
</div>

Cleanr 帮助开发者审阅 `node_modules`、Rust `target`、Xcode 构建产物和包管理器缓存。
它通过键盘驱动的终端界面解释每个候选项，重新校验你的选择，再把选中内容移动到系统
回收站，并保留本地恢复记录。

从一个旧项目开始。浏览器与应用缓存、日志、临时文件和下载文件属于可以额外选择的
范围。各平台覆盖不同，请看[支持与验证矩阵](https://drl990114.github.io/Cleanr/zh-Hans/docs/support-matrix)。

## 首次扫描演示

![Cleanr v0.14.0 对生成的开发者缓存样本进行只读扫描](../../docs/static/img/cleanr-scan.png)

**v0.14.0 · macOS Apple Silicon。** 使用生成的示例项目，只读扫描、查看 Review
并浏览候选项，没有执行清理或恢复。
[观看 34 秒演示](https://drl990114.github.io/Cleanr/media/cleanr-first-scan.mp4)
· [操作说明](https://drl990114.github.io/Cleanr/zh-Hans/docs/demo/)。

## 安装

使用 Node.js 18 或更新版本：

```bash
npm install --global cleanr-cli
cleanr --version
```

也可以使用 Rust 1.98 或更新版本执行 `cargo install cleanr-cli`，或从
[GitHub Releases](https://github.com/drl990114/Cleanr/releases) 下载原生二进制。
[安装、升级、回退与卸载](https://drl990114.github.io/Cleanr/zh-Hans/docs/quick-start)
说明了系统和 CPU 架构的选择方式。

## 选择第一次审阅方式

### 直接使用终端

```bash
cleanr /path/to/project
```

将路径替换为一个真实项目目录。按 `s` 扫描、`r` 审阅、`?` 查看帮助、`q` 退出。
第一次体验无需清理。审阅默认显示候选目录树中最新观测修改时间至少达到 **90 天**的
条目。空结果可能来自文件较新、路径被排除或没有匹配规则，不代表整台电脑已经干净。

看过候选项的原因和风险后，可以用 `space` 调整选择、`c` 打开清理确认框。
`/restore` 打开清理历史。详见[完整入门流程](https://drl990114.github.io/Cleanr/zh-Hans/docs/quick-start)。

### 配合编码 Agent

安装可选的跨 Agent Skill：

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

可以这样提问：“用 Cleanr 审阅这个项目的清理候选项。先解释原因与风险，再由我选择。”
Skill 只会在 CLI 缺失时安装它。底层只读入口为：

```bash
cleanr analyze /path/to/project
```

Cleanr 本身不会上传扫描路径或报告。**运行在本机的 Agent 仍可能把工具输出发送给
云端模型。** 交给 Agent 之前应确认其数据策略；希望不经过 AI 服务时，可直接使用 TUI。
详见[证据与隐私](https://drl990114.github.io/Cleanr/zh-Hans/docs/evidence-and-privacy)。

## 能做什么

- 展示候选原因、大小、置信度和风险；依据可信规则和观测修改年龄进行保守选择。
- 执行前校验、重叠目标检查、系统回收站，以及本地清理和恢复清单。
- 英文与简体中文界面、声明式规则插件，以及 macOS、Linux、Windows 原生包。
- 带版本的 `analyze` JSON 报告，以及用于用户审阅并明确授权计划的独立、带摘要校验
  的 `clean` 命令。

**回收站是可恢复存储，不等于立即释放空间。** 移入后，文件通常仍然占用磁盘。
候选大小和已移动字节数不等于实测的可用空间增量。清空回收站是用户的另一个决定，
也会使 Cleanr 失去恢复来源。恢复尽力而为，不会覆盖已经存在的路径。

`--authorized-by-user` 表示调用方声明已经取得用户授权，Cleanr 无法独立确认是谁
给出了授权。摘要与重扫校验保护通过 Cleanr 命令执行的已审阅计划，但不是约束具有
其他文件操作工具的 Agent 的操作系统沙箱。

## 版本和帮助

文档跟随仓库更新。标为 **Unreleased / 未发布** 的功能，包括分类筛选及相应全选快捷键，
需要后续发行版本；请结合 `cleanr --version` 和[更新记录](../../CHANGELOG.md)阅读。

- [安全与恢复](https://drl990114.github.io/Cleanr/zh-Hans/docs/safety-and-recovery)
- [故障排查](https://drl990114.github.io/Cleanr/zh-Hans/docs/troubleshooting)
- [支持与反馈](../../SUPPORT.md) · [安全问题报告](../../SECURITY.md)
- [发布准备与平台验证](https://drl990114.github.io/Cleanr/zh-Hans/docs/support-matrix)

## 致谢与许可证

Cleanr 包含改编自 [Byron/dua-cli](https://github.com/Byron/dua-cli) 的代码，原项目由
Sebastian Thiel 及贡献者以 MIT 许可证发布。Cleanr 使用 [MIT License](../../LICENSE)。
