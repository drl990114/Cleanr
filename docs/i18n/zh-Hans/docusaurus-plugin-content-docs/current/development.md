---
description: 作为贡献者构建、测试、检查和维护 Cleanr 文档。
---

# 开发

## 环境要求

- Rust 1.94.1 或兼容的更高版本
- 文档站点需要 Node.js 20 或更高版本
- pnpm 10

## 构建 workspace

构建 workspace：

```bash
cargo build
```

构建发布二进制：

```bash
cargo build --release
```

CLI 二进制位于 `target/release/cleanr`。

## 运行与 CI 相同的检查

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --all-targets --all-features --locked
```

校验生成的 JSON Schema：

```bash
cargo run --locked -p cleanr-cli -- plugin schema manifest >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema rules >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema language >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema config >/dev/null
```

## 测量本地扫描性能

被忽略的文件系统基准只扫描显式传入的根目录，并输出汇总耗时、条目数、错误数和
字节数。它不会输出单条路径，也不会随常规测试运行。

```bash
CLEANR_BENCH_ROOT=/path/to/local/fixture \
CLEANR_BENCH_ROUNDS=5 \
CLEANR_BENCH_WORKERS=1 \
cargo test -p cleanr-fs --locked --test scan_performance -- \
  --ignored --nocapture
```

扫描器改动前后应使用相同 fixture、文件系统状态、构建配置和冷热缓存条件。开发机
结果不能直接作为跨平台发布性能结论。worker 数量大于 `1` 时运行的是内部实验后端，
不是用户配置。只有在重复运行的报告指纹一致、P95 有实质改善，并且独立测得的峰值 RSS
不超过串行基线的 `1.25x` 时，才应考虑公开或默认启用。macOS 上应让编译后的测试
可执行文件直接运行在 `/usr/bin/time -l` 下，以排除 Cargo 和编译器内存；基准输出的
`rss_after_kib` 只是扫描后快照，不是峰值 RSS。

内存中的证据、计划和 JSON 序列化阶段可使用合成的忽略基准；其中只会生成 fixture
名称，不会包含本地文件系统路径。

```bash
CLEANR_BENCH_ENTRIES=100000 \
CLEANR_BENCH_ROUNDS=5 \
cargo test -p cleanr-core --locked --test pipeline_performance -- \
  --ignored --nocapture
```

TUI 的忽略基准会先构造大型合成候选集并完成预热，只测量 `TestBackend` 的 draw 调用，
输出平均、P95 和最大帧耗时，但不设置依赖机器性能的通过阈值：

```bash
CLEANR_BENCH_CANDIDATES=10000 \
CLEANR_BENCH_FRAMES=200 \
cargo test -p cleanr-tui --locked \
  scan_view_render_performance -- --ignored --nocapture
```

## 本地运行文档站点

```bash
cd docs
pnpm install
pnpm start
```

默认开发地址为 `http://localhost:3000/`。

提交文档改动前：

```bash
pnpm typecheck
pnpm build
```

## 保持中英文同步

- 英文源文档位于 `docs/docs/`。
- 简体中文文档位于
  `docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`。
- 共享 UI 文案位于 `docs/i18n/zh-Hans/` 下的 locale JSON 文件。

修改 React 翻译文本、导航、页脚或侧边栏分类后，重新生成翻译键：

```bash
pnpm docusaurus write-translations --locale zh-Hans
```

翻译新增条目，并构建两个语言版本。

## 贡献检查清单

- 行为变化需要新增或更新测试。
- 命令、默认值、安全行为或平台支持变化时，更新用户文档。
- 同一次改动中更新英文和简体中文。
- 示例应可执行，不要把计划中的行为写成已经实现。
- 运行格式化、Clippy、workspace 测试、类型检查和文档构建。
