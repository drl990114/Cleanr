---
description: 安全地使用 Cleanr 带版本的本地分析报告，并交给外部本地 AI 工具。
---

# 证据与隐私

Cleanr 的 AI 友好方式是暴露确定性的本地事实，而不内置模型。`cleanr analyze` 是
只读证据边界。另一个受限的 `cleanr clean` 命令只有在用户明确授权后，才能把某个
已经审阅的确切计划移动到系统回收站。

## 本地分析契约

对一个或多个根目录运行分析：

```bash
cleanr analyze /path/to/project
```

它也支持 `--global`，以及可重复使用的 `--global-kind <kind>`，用于已知的用户级
清理位置。推荐策略来自共享配置：

```toml
[recommendations]
preselect_after_days = 90
```

将 `preselect_after_days` 设为 `0` 可关闭年龄门槛，也可设为 `1` 到 `3650` 之间的
整数。TUI、`cleanr analyze`、`cleanr plan` 和 `cleanr dry-run` 使用同一项策略。

命令会把带版本的 `AnalysisReport` JSON 写到标准输出。它只扫描和评估证据，**不会**
创建清理计划、修改当前 TUI 选择、请求清理授权或移动文件。

## 安装 Agent Skill

仓库提供跨 Agent 的 `cleanr-review-disk-cleanup` Skill，用于本地证据审阅，以及
经过明确授权的可恢复清理。
使用开放的 [Skills CLI](https://github.com/vercel-labs/skills) 直接从 GitHub 安装：

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

安装器会检测本机支持的 Agent，并让你选择安装目标。`-g` 表示安装到用户级、供所有
项目使用；去掉 `-g` 则只安装到当前项目。也可以使用 `-a <agent-name>` 明确指定
Agent。

安装后请在选定的 Agent 中新建任务或会话。支持显式调用 Skill 的 Agent 可使用
`$cleanr-review-disk-cleanup`；其他 Agent 可以直接要求“审阅 Cleanr 磁盘清理证据”。
该 Skill 使用可移植的 `SKILL.md` 格式，并不专属于 Codex，可安装到 Skills CLI 支持的
任意 Agent。Skill 默认保持只读。只有当前用户看过计划摘要，并明确授权该确切计划和
SHA-256 后，它才允许执行。执行使用系统回收站和本地清单，不会永久删除。

## 报告的含义

同一报告拥有固定的 `as_of` 时间，因此年龄判断在门槛边界上保持一致。报告包括：

- schema 和分析标识符、策略快照和完成时间；
- 扫描根目录、完整性状态和结构化扫描问题；
- 每个候选项的报告作用域不透明 ID、本地路径、大小、类型和回收方式；
- 修改时间证据、覆盖范围、规则匹配和重叠处理结果；
- 确定性的推荐状态和决策代码，既解释推荐，也解释未预选。

修改时间是观测到的文件系统元数据，并不等于用户最后访问文件的证据。对于目录，
Cleanr 会考虑已扫描后代中最新的观测修改时间。缺失、未来、部分或不完整的证据都会
阻止自动预选。

## 推荐的外部 Agent 工作流

1. 本地 Agent 对用户认可的范围调用 `cleanr analyze`。
2. 它读取报告，提出问题、解释或建议审阅顺序。
3. 如果用户希望清理，它使用 `cleanr plan --output` 写入本地计划，检查已选的
   trash 动作，并汇总确切根目录、数量、大小、风险、路径和输出的 SHA-256。
4. 当前用户在看到摘要后明确授权该确切计划。
5. Agent 使用计划路径、已审阅 SHA-256 和 `--authorized-by-user` 运行
   `cleanr clean`。
6. Cleanr 校验摘要、重新扫描并比较计划、逐项校验目标，把成功条目移动到系统
   回收站并记录清单。

分析命令没有清理操作。建议、推荐、最初的清理请求或宽泛的长期授权都不是执行令牌。
计划发生变化时，Agent 必须重新生成、汇总并取得新计划的授权。

## 数据边界

`AnalysisReport` 和清理计划文件都是刻意设计成**本地**的契约。它们包含原始本地
路径、扫描根目录、规则原因和风险说明，以及问题路径。Cleanr 没有内置 AI Provider、
API Key 设置、prompt 传输或会将这些内容发送到其他地方的遥测。

不要原样把 JSON 转发给远程服务。如果选择分享其中任何内容，应自行缩小范围并去除
敏感细节。安全的远程分享功能需要独立的脱敏 DTO 和明确的威胁模型审查；本地报告
不是这种 DTO。
