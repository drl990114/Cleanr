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

对于常规 macOS 审阅，除非用户把开发者缓存纳入范围，否则应将其分开：

```bash
cleanr analyze --global \
  --global-kind browser-caches \
  --global-kind app-caches \
  --global-kind logs \
  --global-kind temp-files
```

只有用户认可该范围后，才可以把相同的 `global-kind` 参数用于
`cleanr plan --output`。需要检查 Homebrew、包管理器和 Xcode 目标时，再明确加入
`developer-caches`。只有用户明确要求审阅 Downloads 中的个人文件时，才加入
`downloads`。

Windows 常规审阅应把默认范围限制在两个保守、仅包含普通文件的分类：

```bash
cleanr analyze --global \
  --global-kind app-caches \
  --global-kind temp-files
```

这个范围会发现当前用户的 Temp 与 DirectX `D3DSCache` 位置。Windows 专属规则只
匹配至少 30 天未修改的普通文件，不会选择这两个目录本身。加入浏览器或开发者缓存前
应另行询问用户。崩溃转储、Explorer 缩略图数据库、Windows Update 数据、Prefetch、
Downloads、注册表数据、回收站和系统所有的根目录不属于这个常规范围。

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
- 对于全局分析，还包括请求的全局类别，以及每个已存在命名位置的类别、标签、本地
  路径和实际覆盖它的扫描根目录；
- 每个候选项的报告作用域不透明 ID、本地路径、大小、类型和回收方式；
- 修改时间证据、覆盖范围、规则匹配和重叠处理结果；
- 确定性的推荐状态和决策代码，既解释推荐，也解释未预选。

修改时间是观测到的文件系统元数据，并不等于用户最后访问文件的证据。对于目录，
Cleanr 会考虑已扫描后代中最新的观测修改时间。缺失、未来、部分或不完整的证据都会
阻止自动预选。

可选的 `scan.global` 对象是 `cleanr.analysis.v1` 的向后兼容附加字段。显式路径分析会
省略它，旧的 v1 报告即使没有该字段也仍可反序列化。不要从 `scan.roots` 猜测全局
覆盖：父目录会为提高扫描效率而去重，而 `scan.global.locations` 会把每个命名位置映射到
一个自然遍历结束的覆盖根。结构化扫描问题仍会指出按配置忽略或跨文件系统边界跳过的子树，
因此仅有位置记录并不代表每个后代都已读取。仅当扫描完整性为 complete 时，请求类别没有
位置才表示 Cleanr 没有发现该类别的已知现存位置；这仍不表示电脑已经干净。仓库内置
Agent Skill 会把这些证据转换为 `found-candidates`、
`checked-empty`、`no-known-location` 或 `partial`；对于 Cleanr 不应执行的系统更新和
其他系统所有工作，则使用 `os-managed`。

## 推荐的外部 Agent 工作流

1. 本地 Agent 对用户认可的范围调用 `cleanr analyze`。
2. 它读取报告，提出问题、解释或建议审阅顺序。
3. 如果用户希望清理，它使用 `cleanr plan --output` 写入本地计划。只有当前用户在
   审阅证据后对确切候选路径作出选择时，才可以用可重复的 `--select` 和
   `--deselect` 记录这些选择；不要直接编辑计划文件。
4. Agent 检查已选的 trash 动作，并汇总确切根目录、数量、大小、风险、计划路径和
   输出的 SHA-256。
5. 当前用户在看到摘要后明确授权该确切计划。
6. Agent 使用计划路径、已审阅 SHA-256 和 `--authorized-by-user` 运行
   `cleanr clean`。
7. Cleanr 校验摘要、重新扫描并比较计划、逐项校验目标，把成功条目移动到系统
   回收站并记录清单。

分析命令没有清理操作。建议、推荐、最初的清理请求或宽泛的长期授权都不是执行令牌。
Agent 不能仅凭自己的判断选中需审阅候选项；未知、被重叠抑制或被安全策略排除的路径
无法选中。计划发生变化时，Agent 必须重新生成、汇总并取得新计划的授权。

## 数据边界

`AnalysisReport` 和清理计划文件都是刻意设计成**本地**的契约。它们包含原始本地
路径、扫描根目录、规则原因和风险说明，以及问题路径。Cleanr 没有内置 AI Provider、
API Key 设置、prompt 传输或会将这些内容发送到其他地方的遥测。

## 预算受限证据

扫描达到预算时，`scan.budget_exceeded` 会记录不含路径的上限和观测值，报告完整性为
`partial`，候选覆盖状态为 `unknown`。已收集的本地证据仍可用于审阅，但它只能读取：
Cleanr 会拒绝据此生成清理计划。必须完成一次新的完整扫描后才能生成计划或清理。

不要原样把 JSON 转发给远程服务。如果选择分享其中任何内容，应自行缩小范围并去除
敏感细节。安全的远程分享功能需要独立的脱敏 DTO 和明确的威胁模型审查；本地报告
不是这种 DTO。
