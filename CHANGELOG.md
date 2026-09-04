# Changelog

Release notes describe shipped behavior. This file also tracks repository
changes that are not yet part of an installable release. Entries under
Unreleased are pending release; installing the latest package does not include
later source or CI fixes.

## Unreleased

## 0.15.0

- Show cleanup categories, filter candidates with `f`, retain selections across
  filters, and use `Shift+A` for true global select-all. `a` / `%` apply to the
  current filter; hidden selections remain visible in totals and confirmation.
- Prepare corrected `/Cleanr/` documentation URLs, a read-only first walkthrough,
  platform and installation guidance, support forms, and a private security
  reporting route.
- Clarify that Trash bytes are not immediate free space, external agents may
  send tool output to cloud models, and the approval flag is a caller assertion.
- Add a real v0.14.0 read-only terminal recording, generated sample fixtures,
  and bilingual reproduction steps.
- Update vulnerable locked dependencies (`crossbeam-epoch`, `quinn-proto`, and
  `anyhow`). Remaining Ratatui dependency warnings are documented in SECURITY.md.
- Require the same commit to pass cross-platform source checks and isolated
  installation smoke tests before publishing. Ship checksums, build provenance,
  and per-platform verification evidence; keep npm tarballs identical between
  testing and publishing.
- Write `cleanr.restore.v2` recovery records while retaining read support for v1.
  Persist each item separately, distinguish `not-attempted` from `pending`, and
  stop before another cleanup item when recording a result fails. An OS file
  lock protects the recovery state. After an interrupted `pending` operation,
  inspect the original path and system Trash before recovery. Do not use an
  older binary to process v2 records.
- Explicitly release operation locks when an operation ends, avoiding temporary
  lock contention caused by file handles inherited during concurrent process creation.

### 简体中文

- 新增分类标签、`f` 筛选、跨筛选累积选择、`Shift+A` 全局全选；`a` / `%` 只作用于
  当前筛选范围，隐藏选择仍出现在汇总与确认中。
- 修正 `/Cleanr/` 文档路径，补齐只读首次体验、平台安装与升级说明、反馈和私密安全
  报告入口，并明确回收站空间、云端 Agent 与授权声明的边界。
- 提供 v0.14.0 真实只读终端演示、生成的示例项目及中英文复现步骤。
- 修复 `crossbeam-epoch`、`quinn-proto`、`anyhow` 的锁定依赖问题；Ratatui 剩余依赖
  告警与升级限制记录在 SECURITY.md。
- 发布前要求同一提交通过三平台源码检查及隔离安装验证，提供校验和、构建来源与
  平台验证报告；npm 测试和发布使用同一份安装包。
- 恢复记录升级为 `cleanr.restore.v2`，继续读取 v1。逐项持久化并用系统文件锁保护
  状态；`not-attempted` 表示尚未调用回收站，`pending` 表示结果可能尚未落盘。
  写入结果失败会停止后续条目；中断后应人工核对原路径和系统回收站，勿用旧二进制
  处理 v2 记录。
- 操作结束时显式释放文件锁，避免并行创建进程时继承的句柄短暂阻碍后续操作。

Validation and compatibility details: [support matrix](https://drl990114.github.io/Cleanr/docs/support-matrix).

## 0.14.0

- See the original [v0.14.0 release](https://github.com/drl990114/Cleanr/releases/tag/v0.14.0).
The release-time source CI had two Windows TUI assertion failures; later source
fixes do not change those already published assets. See the verification matrix
before treating a platform as end-to-end validated.

### 简体中文

v0.14.0 对应源码 CI 曾有两个 Windows TUI 断言失败，后续源码修复不改变已发布资产。
版本兼容性与平台证据见[支持矩阵](https://drl990114.github.io/Cleanr/zh-Hans/docs/support-matrix)。
