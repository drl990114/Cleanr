---
description: 面向贡献者的 Cleanr crate、数据流和安全边界说明。
---

# 架构

本页面面向需要理解 Cleanr 内部职责的贡献者和插件作者。只想使用应用时，请从
[使用 Cleanr](./using-cleanr.md)开始。

## Workspace crate

| Crate | 路径 | 职责 |
| --- | --- | --- |
| `cleanr-core` | `crates/core` | 扫描条目、规则命中、证据报告、清理计划、安全策略和清单模型 |
| `cleanr-cli` | `crates/cli` | 命令行适配、参数解析、只读输出、分发命令和插件管理 |
| `cleanr-tui` | `crates/tui` | 终端应用、状态机、页面和后台工作流适配 |
| `cleanr-fs` | `crates/fs` | 文件系统扫描、元数据收集、取消和 `ScanReport` 生成 |
| `cleanr-rules` | `crates/rules` | 内置与插件规则加载、校验、匹配和 `RuleRegistry` |
| `cleanr-plugin-api` | `crates/plugin-api` | 带版本 manifest、发现、兼容性、信任、Schema 和诊断 |
| `cleanr-config` | `crates/config` | 配置 Schema、默认值、校验和原子写入 |
| `cleanr-i18n` | `crates/i18n` | 纯语言包解析、校验、回退和运行时语言切换 |
| `cleanr-tasks` | `crates/tasks` | 共享扫描/证据/计划工作流、受保护的清理入口、恢复、平台回收站适配和清单持久化 |

## 运行时数据流

```text
CLI 或 TUI 适配 + 配置
             │
             ▼
      cleanr-tasks 工作流
解析范围 → 扫描 → 规则 → 证据 → 计划
   │        │      │       │
   │        │      │       └── cleanr-core
   │        │      └────────── cleanr-rules
   │        └───────────────── cleanr-fs
   ▼
用户审阅
   │          │
   │          └── 委托执行：精确摘要 + 重扫 + 来源校验
   └───────────── 本地 TUI：明确确认
                         │
                         ▼
pending 清单 → 目标重校验 → 系统回收站 → 清单更新
                         │
                         └──────────────→ 恢复 → 恢复清单
```

计划生成器会先移除重叠候选项，再计算已选空间和候选项总空间。

仅接收条目列表的 `build_cleanup_plan*` 函数保留为已弃用的兼容 API。产品代码必须
通过共享工作流使用基于分析报告的构建器，以保留扫描完整性和来源信息。

`cleanr-tasks` 导出的共享工作流是解析范围、扫描、规则标注、生成证据和计划的唯一产品级
编排层。CLI 与 TUI 只适配参数、进度和展示，不再各自组合底层 crate。

## 内部模块边界

- `cleanr-core` 分离序列化模型、证据、计划、安全策略和执行/恢复清单；
- `cleanr-fs` 分离范围发现、扫描遍历、预算记账和平台文件身份；
- `cleanr-rules` 分离 Schema、插件加载、Registry/索引所有权和匹配；
- `cleanr-tasks` 分离工作流编排、清理、恢复、清单存储和操作系统适配。

CI 中的 `node scripts/check-architecture.mjs` 会守住这些边界：禁止 CLI/TUI 绕过
共享工作流，禁止公开原始清理执行器，并禁止 `cleanr-i18n` 引入仅分发阶段需要的
网络依赖。

## TUI 边界

`cleanr-tui` 将渲染与 I/O 分离：

- `app/` 负责状态变化和用户动作；
- `effects/` 负责后台扫描、持久化、清理和恢复工作；
- `views/` 只根据应用状态渲染；
- `commands/` 将动作请求映射到命令面板；
- `terminal.rs` 负责 raw mode、输入轮询、绘制和终端恢复。

页面不会遍历文件系统。后台任务将结果发送回状态机，因此取消和部分失败都能
明确反映在 UI 中。

## 外部本地 AI 边界

`cleanr analyze` 是供同一台机器上的外部 Agent 使用的只读 CLI 边界。它扫描、
应用确定性的规则和推荐策略，并输出带版本的 `AnalysisReport` JSON；不会创建
清理计划、授予授权或移动文件。Agent 可以基于证据解释结果或提出审阅建议，
但仍由用户在 Cleanr 中选择并确认清理。

报告包含原始本地路径、扫描根目录、规则元数据和解释性文本，以及诊断信息。它刻意
是本地契约，而非远程传输对象；未来若需要远程分享，必须另行设计脱敏 DTO 和威胁
模型。

## 安全边界

安全性由多个层次共同执行：

- `cleanr-rules` 只允许高置信度可信规则自动选择；
- `cleanr-core` 在生成计划时排除受保护和重叠候选项，并为选中目录记录指纹；
- `cleanr-tasks` 分别提供本地确认与委托清理入口，原始执行器保持 crate-private；
  移动文件前会写入 journal，并在执行时重新校验每个目标；
- 委托清理将授权绑定到已审阅计划的精确 SHA-256 摘要，重建保存的扫描范围和
  推荐策略，重新扫描，并在计划或来源漂移时拒绝执行；
- 回收站后端在平台支持时记录回滚信息；
- `cleanr analyze` 只读，不能创建清理授权或调用清理；
- 这个接口不会把扫描证据交给内置模型或 Provider。

插件默认保持声明式。manifest、规则和翻译只会作为数据解析；动态 hook 是单独
受信任的外部命令能力。

## 持久化数据

配置使用平台配置目录。清理和恢复清单位于平台状态目录下的 `cleanr/`，
分别存放在 `runs/` 和 `restores/` 中。

`cleanr-tasks` 通过 `ManifestRepository` 统一负责清单持久化，把列表、查找和
原子写入集中成一套供 TUI 与 CLI 共用的 API。

写入使用临时文件和原子替换，避免写入中断时静默破坏有效配置或清单。

Agent 可以在本机执行工具，同时把输出发送给云端模型。授权参数是调用方声明，
不是独立的人类身份认证或操作系统沙箱。详见[证据与隐私](./evidence-and-privacy.md)。
