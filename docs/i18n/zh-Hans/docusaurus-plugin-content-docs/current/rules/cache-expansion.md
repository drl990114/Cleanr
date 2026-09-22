---
title: 开发与 AI 缓存覆盖
---

本页说明 **Cleanr v0.17.0** 新增的缓存覆盖；旧版本不包含这些规则和保护。规则仅识别标准用户位置，
新增候选默认不选中，交由人工复核，也不会调用工具自身的清理命令。共享的未修改天数
筛选仍然生效；修改时间不能证明运行时、模型或环境已经无人使用。

## 可复核的位置

`~` 表示用户主目录，`%LOCALAPPDATA%` 表示 Windows 本地应用数据目录。

| 家族 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Playwright | `~/Library/Caches/ms-playwright` | `~/.cache/ms-playwright` | `%LOCALAPPDATA%/ms-playwright` |
| Puppeteer | `~/.cache/puppeteer` | `~/.cache/puppeteer` | `~/.cache/puppeteer` |
| Electron | `~/Library/Caches/electron` | `~/.cache/electron` | `%LOCALAPPDATA%/electron/Cache` |
| Cypress | `~/Library/Caches/Cypress` | `~/.cache/Cypress` | `%LOCALAPPDATA%/Cypress/Cache` |
| Go build | 沿用原有 macOS 规则 | `~/.cache/go-build` | `%LOCALAPPDATA%/go-build` |
| pip | 沿用原有 macOS 规则 | 沿用原有 Linux 规则 | `%LOCALAPPDATA%/pip/Cache` |
| sccache | `~/Library/Caches/Mozilla.sccache` | `~/.cache/sccache` | `%LOCALAPPDATA%/Mozilla/sccache` |
| TorchInductor / 嵌套 Triton | — | `/tmp/torchinductor_<user>` 及其 `triton` 子目录 | — |
| JetBrains 索引 | `~/Library/Caches/JetBrains/<产品版本>/index` | `~/.cache/JetBrains/<产品版本>/index` | `%LOCALAPPDATA%/JetBrains/<产品版本>/index` |
| JetBrains 日志 | `~/Library/Logs/JetBrains/<产品版本>` | 产品系统目录内的 `log` 子目录 | 产品系统目录内的 `log` 子目录 |
| Conda 压缩下载包 | 下文列出的标准主目录包缓存 | 相同 | 相同 |
| Hugging Face Xet 分块 | `~/.cache/huggingface/xet/<环境>/chunk_cache` | 相同 | 相同 |

浏览器二进制文件可能被多个项目共享。请先审核保留版本、离线需求并退出相关工具；
之后运行可能需要大量下载。编译缓存可能触发耗时的重编译或 GPU 调优。sccache 有后台
服务，仅关闭终端不一定会停止服务。

TorchInductor 仅在系统临时目录下有限展开直接命名为 `torchinductor_*` 的目录。
通过系统用户 ID 检查实际归属，不信任目录名中的用户名或环境变量，并在执行前再次
检查归属。规则匹配标准 `/tmp` 布局，不承诺支持自定义临时目录。父目录与嵌套 Triton
候选沿用重叠处理：选择其中一个，空间只计一次。

JetBrains 仅覆盖有版本号的 IntelliJ IDEA（`IntelliJIdea`、`IdeaIC`）、PyCharm
（`PyCharm`、`PyCharmCE`）和 WebStorm。只开放生成的 `index` 子目录与诊断日志。
Local History、插件、配置、scratch、未经审核的产品以及未来新增的未知子目录均保持
只读。历史诊断日志移除后不能重新生成。

Conda 只识别 `pkgs` 的直接普通文件 `.conda`、`.tar.bz2`，同次扫描必须看到普通文件
`urls.txt` 标记。标准主目录根包括 `.conda`、`miniconda3`、`anaconda3`、`miniforge3`
及对应首字母大写的安装目录。解压目录、伪装成归档后缀的目录、已安装环境和元数据
都保留。不识别自定义或共享包根，也不会把 Downloads 里的同名文件当作 Conda 缓存。

Xet 支持环境目录下的 `chunk_cache` 以及旧版直接位于 `xet/chunk_cache` 的布局。
模型、数据集、令牌、共享引用、上传 `shard_cache`、可续传 `staging` 和未知状态均
保持只读。下载分块缓存是可选的，新版 hf_xet 默认关闭；不能据此推断用户一定有
这类缓存或能回收多少空间。

## 只读保护与进程检查

uv 缓存改为**只读识别**。上游要求使用 uv 自身缓存管理和锁机制，直接操作文件系统
不安全。保护覆盖 `~/.cache/uv`、旧 macOS 位置 `~/Library/Caches/uv` 和
`%LOCALAPPDATA%/uv`；原有宽泛下载缓存规则不再把 uv 当作可移入回收站的候选。

本地分析保留只读条目的原因、风险、`read_only_scope` 和已有的 `excluded` 状态。
保留路径、包含它的父目录以及受保护的后代不能进入清理计划。禁用内置清理包也会保留其只读保护。保护不依赖规则优先级、
手动选择或推荐状态，即使单独扫描受保护子树内部的候选也会生效。`entry` 保护混合
数据容器，同时允许明确审核的子项；`subtree` 保留全部后代。执行器会在建立清理日志
前拒绝被选中的只读规则证据。

运行时检查识别已知进程名和可执行文件名，也覆盖 Python 版本后缀。进程快照不可用、
或必要的归属检查无法完成时，候选不可选。这些检查不能证明任意改名、嵌入式或稍后
启动的程序不会访问缓存；清理前仍需停止相关任务。Cleanr 不终止进程、不执行项目
配置，也不执行 Docker 清理、模型库存删除或 `uv cache clean`。

全局位置使用开发缓存类别，JetBrains 诊断日志使用日志类别。遍历根合并后，最具体
位置仍保留类别隔离。自定义环境变量和应用配置不会被执行，也不会因此扩展出未经
审核的位置支持。

## 来源与验证边界

2026-09-22 对照上游文档核查路径与数据性质。实现复用 globset 匹配器、同次扫描的
标记证据、sysinfo 进程检查和 Rustix 系统用户 ID；没有新增清理后端或应用命令执行。

- [Playwright 浏览器管理](https://playwright.dev/docs/browsers)
- [Puppeteer 配置](https://pptr.dev/guides/configuration)
- [Electron 下载缓存](https://www.electronjs.org/docs/latest/tutorial/installation#cache)
- [Cypress 二进制缓存](https://docs.cypress.io/app/references/advanced-installation#binary-cache)
- [pip 缓存](https://pip.pypa.io/en/stable/topics/caching/)
- [Go 构建与测试缓存](https://go.dev/cmd/go/#hdr-Build_and_test_caching)
- [sccache 本地缓存](https://github.com/mozilla/sccache/blob/main/docs/Local.md)
- [PyTorch 编译缓存配置](https://docs.pytorch.org/tutorials/recipes/torch_compile_caching_configuration_tutorial.html)
- [JetBrains 目录含义](https://www.jetbrains.com/help/idea/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html)及 [PathManager 中的索引路径](https://github.com/JetBrains/intellij-community/blob/master/platform/util/src/com/intellij/openapi/application/PathManager.java)
- [Conda 包缓存清理](https://docs.conda.io/projects/conda/en/stable/commands/clean.html)
- [Hugging Face 缓存结构](https://huggingface.co/docs/huggingface_hub/guides/manage-cache)
- [uv 缓存安全](https://docs.astral.sh/uv/concepts/cache/#cache-safety)

测试使用生成的临时目录、实际扫描器、规则引擎、分析和计划函数，以及注入的进程
快照与假回收站后端。跨平台路径夹具不等于 Windows/Linux 实机验证，也不证明真实
GPU/IDE 重建、释放空间或用户需求。这些测试不会扫描或移动真实用户缓存。
