---
description: 使用生成的示例项目，观看真实的 Cleanr 只读终端操作。
---

# 首次扫描演示

首页演示使用已发布的 **Cleanr v0.14.0，运行于 macOS Apple Silicon**。
这段约 34 秒的真实终端操作使用生成的示例项目，依次扫描、打开 Review、
浏览候选项、查看帮助并退出，没有确认清理，也没有执行恢复。

## 演示中的操作

| 时间 | 操作 | 观察重点 |
| --- | --- | --- |
| 0–3 秒 | 打开示例项目目录。 | 从一个范围明确的目录开始。 |
| 3–12 秒 | 按 `s` 扫描。 | 两个候选项共 24 MiB：16 MiB 的 `node_modules` 和 8 MiB 的 Rust `target`。选择前先看命中原因、时间和风险。 |
| 12–18 秒 | 按 `r` 打开 Review。 | 查看已选候选项；打开 Review 不会执行清理。 |
| 18–26 秒 | 用 `j`、`k` 浏览。 | 查看两个候选项及其详情。 |
| 26–30 秒 | 按 `?` 打开帮助。 | 继续前先了解可用操作。 |
| 30–34 秒 | 关闭帮助并退出。 | 示例文件保持不变。 |

示例还包含一个本次未命中的 4 MiB `dist` 文件。所有示例的修改时间设为
120 天前；录制脚本在退出后核对文件内容和修改时间。
**24 MiB 是候选项大小，不是已释放的磁盘空间。**
这段演示不验证系统回收站和恢复后端，相关边界请看
[安全与恢复](./safety-and-recovery.md)及[平台验证记录](./support-matrix.md)。

分类筛选与 `Shift+A` 适用于 **0.15.0 及后续版本**，未出现在这段 v0.14.0 演示中。

## 复现录制

准备可信的 Cleanr 二进制、`uv` 提供的 Python、`ffmpeg` 和等宽字体。
脚本使用 macOS 或 Linux 的 POSIX 终端，不适用于 Windows 录制。
在仓库根目录运行：

```bash
uv run scripts/record-demo.py --binary /absolute/path/to/cleanr --output /tmp/cleanr-demo
```

Linux 还需传入 `--font /absolute/path/to/monospace.ttf`。
脚本自行生成临时项目，只发送只读操作和导航按键，结束时移除临时示例。
输出目录中会保存 PNG、MP4、原始 ANSI `.cast`、扫描纯文本及版本、哈希元数据。
使用新目录可保留之前的录制结果。

首页素材位于 `docs/static/img/cleanr-scan.png` 和 `docs/static/media/`。
其中 `cleanr-demo.json` 记录二进制 SHA-256 和执行环境。
录制源码是 `scripts/record-demo.py`，终端画面由 pyte 和 Pillow 根据实际
ANSI 输出渲染。

要对自己的项目进行只读扫描，请看[快速开始](./quick-start.md)。
