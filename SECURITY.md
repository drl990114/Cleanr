# Security reporting

Report a suspected safety-check bypass, unauthorized cleanup path, sensitive
report exposure, or exploitable plugin behavior through GitHub's
[private vulnerability report](https://github.com/drl990114/Cleanr/security/advisories/new).
Private reporting is enabled. Do not post exploit details or personal paths in
public issues. If GitHub does not offer that route, open a public issue asking
only for a private reporting contact, without technical details.

Include the Cleanr version, OS, installation method, expected protection, and a
minimal reproduction using generated files. Do not attach original analysis
reports, plans, manifests, credentials, or real personal paths. A maintainer may
request a minimized sample through the private channel.

The latest release and current source are investigated; no response-time or
long-term support guarantee is established. Older-version fixes are assessed
case by case. [Release readiness](https://drl990114.github.io/Cleanr/docs/support-matrix)
records current version and platform evidence.

Cleanr's plan digest and revalidation protect its execution path. The
`--authorized-by-user` flag is the caller's assertion, not independent human
authentication or an OS sandbox. Recovery depends on system Trash and manifests;
it is not a backup guarantee. Routine installation and rule questions belong in
[Support](SUPPORT.md).

## 安全问题报告

怀疑存在安全校验绕过、未经授权的清理路径、敏感报告泄露或可利用的插件行为时，请使用
[GitHub 私密漏洞报告](https://github.com/drl990114/Cleanr/security/advisories/new)。该入口已启用。
不要在公开 issue 发布利用细节或个人路径；入口不可用时，只公开请求一个私密联系方式。

请提供版本、系统、安装方式、预期保护，以及使用生成文件的最小复现。不要上传原始分析
报告、计划、清单、凭证或真实个人路径。目前没有响应时限或长期支持承诺；旧版本修复
逐案评估。普通安装和规则问题请查看[支持与反馈](SUPPORT.md)。

## Dependency audit snapshot — 2026-09-04

The current source lockfile was audited against RustSec database commit
`5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5` (updated 2026-09-02), after updating
`crossbeam-epoch` to 0.9.20, `quinn-proto` to 0.11.15, and `anyhow` to 1.0.103.
The audit reported **zero vulnerability entries and three informational
warnings**, with no ignored advisories. This describes the source lockfile;
already-installed releases do not receive these fixes automatically.

All three remaining warnings enter through `ratatui 0.29.0`:

| Advisory | Locked dependency | Remaining issue and available fix |
| --- | --- | --- |
| [RUSTSEC-2026-0002](https://rustsec.org/advisories/RUSTSEC-2026-0002.html) | `lru 0.12.5` | Unsound mutable iteration; fixed in `lru >= 0.16.3`. |
| [RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html) | `lru 0.12.5` | `LruCache::pop()` is not panic-safe; fixed in `lru >= 0.18.2`. |
| [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436.html) | `paste 1.0.15` | The procedural-macro crate is unmaintained; no patched `paste` release is listed. |

Source inspection found Ratatui's private layout cache using `new`, `resize`,
and `get_or_insert`, with no calls to the affected `IterMut` or `pop` APIs.
Cleanr's release profile also uses `panic = "abort"`, whereas the `pop` advisory
requires a panicking key destructor followed by continued use after unwinding.
These observations narrow the known trigger paths; they do not establish that
the dependency is sound or remove the warnings. The unmaintained `paste`
dependency is a build-time maintenance risk, not a reported runtime exploit.

Ratatui 0.29 requires `lru ^0.12` and `paste ^1`; the current registry has no
stable compatible patched versions. Resolving these warnings requires a
separate Ratatui upgrade review, including its split-crate API, layout behavior,
terminal backends, and the resulting dependency audit. Do not suppress the
advisories or describe this snapshot as a clean security bill. Re-run the audit
against the exact lockfile used for each release.

## 依赖审计快照 — 2026-09-04

当前源码锁文件已将 `crossbeam-epoch` 升级到 0.9.20、`quinn-proto` 升级到 0.11.15、
`anyhow` 升级到 1.0.103。使用上述 RustSec 数据库提交审计后，结果是 **0 条漏洞记录、
3 条信息性告警**，没有忽略任何 advisory。该结果只对应源码锁文件，不会自动修复已安装版本。

剩余告警都经由 `ratatui 0.29.0` 引入：`lru 0.12.5` 的
[RUSTSEC-2026-0002](https://rustsec.org/advisories/RUSTSEC-2026-0002.html)
涉及可变迭代器的内存安全问题，修复版本为 0.16.3 及以上；
[RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html)
涉及 `LruCache::pop()` 的 panic 安全性，修复版本为 0.18.2 及以上。
`paste 1.0.15` 的
[RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436.html)
表示上游停止维护，目前没有列出的修复版本。

源码检查显示，Ratatui 的私有布局缓存使用 `new`、`resize` 和 `get_or_insert`，
没有发现对受影响 `IterMut` 或 `pop` API 的调用。Cleanr 发布配置使用
`panic = "abort"`，也不满足 `pop` 公告中“键析构发生 panic、展开后继续使用”的条件。
这些只能缩小已知触发路径，不能证明依赖本身没有问题。`paste` 是构建期维护风险，
该公告没有报告运行时利用方式。

Ratatui 0.29 限定 `lru ^0.12` 和 `paste ^1`，目前没有稳定的兼容补丁版本。
后续需要单独评估 Ratatui 升级，包括拆分后的 crate API、布局、终端后端和更新后的依赖审计；
不能通过忽略告警来消除问题。每次发布都应重新审计实际使用的锁文件。
