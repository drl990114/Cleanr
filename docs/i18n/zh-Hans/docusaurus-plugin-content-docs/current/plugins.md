---
sidebar_position: 7
description: 安装、发布或编写 Cleanr 插件，用于清理规则、翻译和受信任的动态候选 hook。
---

# 插件

Cleanr 插件是带版本的 bundle。默认模型是声明式的：插件通过数据文件添加清理规则
和翻译，因此 Cleanr 可以在加载前完成校验。插件也可以声明 `dynamic-candidates`
hook，但 hook 执行是单独的受信任能力，不会因为普通安装自动启用。

插件也可以声明 `scan-locations`。这些位置必须相对于 `home`、`cache`、
`data-local`、`data`、`temp` 或 `downloads` 等受限基准目录，不能使用绝对路径、
父目录跳转或 glob。只有内置或显式信任的插件可以激活扫描位置；不可信位置只产生
诊断并被忽略。

## 包管理

从官方静态索引安装：

```bash
cleanr plugin search cache
cleanr plugin install example.caches
cleanr plugin list
cleanr plugin update
```

从其他 GitHub 仓库或静态索引安装：

```bash
cleanr plugin install example.caches \
  --github-repo owner/repo \
  --github-ref main

cleanr plugin install example.caches \
  --index-url https://example.com/plugins/index.json
```

常用管理命令：

```bash
cleanr plugin info example.caches
cleanr plugin remove example.caches
cleanr plugin trust example.caches
cleanr plugin untrust example.caches
cleanr plugin doctor
```

默认情况下，Cleanr 会安装到平台配置目录下的 `cleanr/plugins`，记录插件来源索引用于
后续更新，把插件目录加入 `[plugins].dirs`，并启用插件声明的规则包。只有审阅过
bundle 后才使用 `--trust`；受信任的高置信度规则可以默认选中清理项。

## 本地开发

创建插件模板：

```bash
cleanr plugin init ./plugins/example-caches \
  --id example.caches \
  --name "Example cache rules"
```

校验并链接到本机配置：

```bash
cleanr plugin validate ./plugins/example-caches
cleanr plugin link ./plugins/example-caches
cleanr plugin unlink example.caches
```

生成编辑器可用的 Schema：

```bash
cleanr plugin schema manifest > plugin.schema.json
cleanr plugin schema index > plugin-index.schema.json
cleanr plugin schema rules > rules.schema.json
cleanr plugin schema locations > locations.schema.json
cleanr plugin schema language > language.schema.json
cleanr plugin schema config > config.schema.json
```

## 官方索引

官方索引是 `plugins/index.json` 静态 JSON 文件。每个条目包含插件元数据，以及每个
可下载文件的 URL、字节大小和 SHA-256。Cleanr 会先下载到 staging 目录，校验所有
hash，校验 bundle，然后再原子替换到安装目录。

内置规则包单独存放在 `crates/rules/builtin-plugins/`，并编译进 Cleanr。除非未来
有意把它们作为可下载插件发布，否则不会列入 `plugins/index.json`。

生成或检查索引：

```bash
cleanr plugin index \
  --plugin-dir plugins \
  --base-url https://raw.githubusercontent.com/owner/repo/main/plugins

cleanr plugin index --check
```

推荐通过 GitHub PR 发布：

1. 将 bundle 放到 `plugins/<bundle-name>/`。
2. 运行 `cleanr plugin validate plugins/<bundle-name>`。
3. 运行 `cleanr plugin index --check`，或重新生成 `plugins/index.json`。
4. 提交插件文件和生成后的索引，打开 PR。

npm 包或 crates 也可以通过静态 HTTP URL 托管同样的 `plugins/` 目录，但 Cleanr
安装器会消费稳定的 JSON 索引格式，而不是 registry 专属压缩包。

## 最小 Bundle

```text
example-caches/
├── plugin.toml
├── locations/
│   └── global.toml
└── rules/
    └── caches.toml
```

```toml title="plugin.toml"
api_version = "1"
id = "example.caches"
name = "Example cache rules"
version = "1.0.0"
description = "Cleanup rules for Example Tool caches."
cleanr_version = ">=0.1.0"
capabilities = ["rules", "scan-locations"]
categories = ["developer"]
keywords = ["cache"]
```

```toml title="locations/global.toml"
id = "example-global-locations"
version = "1.0.0"

[[locations]]
id = "example-linux-cache"
label = "Example Tool cache"
kind = "app-caches"
platforms = ["linux"]
base = "cache"
relative_path = "example-tool"
```

对于需要解释、但 Cleanr 不应遍历或加入计划的系统维护项，使用
`mode = "os-managed"`。

## 信任和 Hook

新插件默认不受信任。它们的候选项会显示出来，但即使规则声明
`default_selected = true`，也不会默认选中。

```toml
[plugins]
trusted = ["example.caches"]
```

受信任 ID 是插件 manifest ID，不是规则包 ID。信任不会绕过路径校验、保护路径、
移动到回收站行为或本地用户确认。

动态 hook 通过 `dynamic-candidates` capability 声明。当前版本会校验这些声明，
但规则加载时不会执行 hook 命令。未来的运行时会把 hook 当作显式外部命令处理，
使用 JSON stdin/stdout、超时和 host 侧校验。安装 hook、清理前 hook 和清理后
hook 仍不属于第一版 hook 运行时范围。
