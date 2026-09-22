---
title: Developer and AI cache coverage
---

This page describes cache coverage added in **Cleanr v0.17.0**. Older releases do
not include these rules or protections. The rules use standard user locations, leave new
candidates unselected for review, and never invoke a tool's native cleanup command.
The shared inactivity filter still applies; modification age does not prove that
a runtime, model or environment is unused.

## Reviewable cache locations

`~` means the user home, and `%LOCALAPPDATA%` means the Windows local application
data directory. The table lists the default layouts matched by these rules.

| Family | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Playwright | `~/Library/Caches/ms-playwright` | `~/.cache/ms-playwright` | `%LOCALAPPDATA%/ms-playwright` |
| Puppeteer | `~/.cache/puppeteer` | `~/.cache/puppeteer` | `~/.cache/puppeteer` |
| Electron | `~/Library/Caches/electron` | `~/.cache/electron` | `%LOCALAPPDATA%/electron/Cache` |
| Cypress | `~/Library/Caches/Cypress` | `~/.cache/Cypress` | `%LOCALAPPDATA%/Cypress/Cache` |
| Go build | Existing macOS rule | `~/.cache/go-build` | `%LOCALAPPDATA%/go-build` |
| pip | Existing macOS rule | Existing Linux rule | `%LOCALAPPDATA%/pip/Cache` |
| sccache | `~/Library/Caches/Mozilla.sccache` | `~/.cache/sccache` | `%LOCALAPPDATA%/Mozilla/sccache` |
| TorchInductor / nested Triton | — | `/tmp/torchinductor_<user>` and its `triton` child | — |
| JetBrains indexes | `~/Library/Caches/JetBrains/<product-version>/index` | `~/.cache/JetBrains/<product-version>/index` | `%LOCALAPPDATA%/JetBrains/<product-version>/index` |
| JetBrains logs | `~/Library/Logs/JetBrains/<product-version>` | Product system directory's `log` child | Product system directory's `log` child |
| Conda package archives | Standard home package caches, described below | Same | Same |
| Hugging Face Xet chunks | `~/.cache/huggingface/xet/<environment>/chunk_cache` | Same | Same |

Browser binaries can be shared by multiple projects. Review retained versions
and offline requirements before selection; a later run can require large
downloads. Stop the owning tools first. Compiler caches may require expensive
recompilation or GPU autotuning. sccache has a background server, so closing a
terminal alone may not stop it.

TorchInductor discovery expands only direct `torchinductor_*` directories under
the system temporary anchor. It checks native effective-user ownership without
trusting the username in the directory name or environment variables. The rule
matches the standard `/tmp` layout; a custom temporary/cache location is not
claimed as supported. Ownership is checked again immediately before execution.
The parent and nested Triton candidate share the normal overlap handling: select
one, and their bytes are counted once.

JetBrains coverage is limited to versioned IntelliJ IDEA (`IntelliJIdea`, `IdeaIC`),
PyCharm (`PyCharm`, `PyCharmCE`) and WebStorm directories. Only the generated
`index` leaf and diagnostic logs are reviewable. Local History, plugins,
configuration, scratches, unreviewed products and unknown future children remain
read-only. Logs cannot be regenerated after their diagnostic history is removed.

Conda coverage requires regular `.conda` or `.tar.bz2` files directly inside
`pkgs`, with a regular `urls.txt` marker observed in the same scan. Standard home
roots are `.conda`, `miniconda3`, `anaconda3`, `miniforge3`, and their documented
capitalized installation names. Extracted directories, including directories
with archive-looking names, installed environments and metadata are retained.
Custom/shared package roots and arbitrary files in Downloads are not recognized
by this rule.

Xet coverage includes the current environment-specific `chunk_cache` and the
older direct `xet/chunk_cache` layout. Model blobs, datasets, tokens, shared
references, upload `shard_cache`, resumable `staging` and unknown state remain
read-only. Chunk caching is optional and is disabled by default in newer hf_xet
versions; this rule does not imply a cache exists or guarantee meaningful savings.

## Read-only boundaries and process checks

uv caches are **inspection-only**. Upstream requires uv's own cache-management
and locking; direct filesystem cleanup is unsafe. This protects `~/.cache/uv`,
the legacy macOS `~/Library/Caches/uv` location, and `%LOCALAPPDATA%/uv`. The prior
broad download-cache rules no longer treat uv as a trash candidate.

Inspection rules retain their reason and risk in local analysis evidence, with
`read_only_scope` and the existing `excluded` recommendation. A retained path,
its covering ancestors, and retained descendants cannot enter a cleanup plan.
Built-in inspection protections also survive disabling their cleanup pack.
Protection is independent of rule priority, manual selection and recommendation
state. It also applies when scanning a candidate directly inside a retained
subtree. An `entry` inspection protects a mixed container while allowing explicitly
reviewed children; a `subtree` inspection retains every descendant. Execution
rejects selected inspection evidence before creating a cleanup journal.

Runtime guards check known process and executable names, including Python version
suffixes. An unavailable process snapshot or required ownership check blocks
selection. These are conservative checks, not proof that an arbitrary renamed,
embedded or subsequently started application cannot use the cache. Quit all
relevant jobs before cleanup. Cleanr does not terminate processes, execute project
configuration, prune Docker, delete model inventories, or run `uv cache clean`.

Global locations use the developer-cache category; JetBrains diagnostic log
locations use the log category. The most specific location preserves category
isolation even when traversal roots are coalesced. User-defined environment
variables and custom application configuration are not evaluated as executable
configuration or treated as additional verified layouts.

## Sources and verification limits

Paths and retention boundaries were reviewed against upstream documentation on
2026-09-22. The implementation uses the existing globset matcher, snapshot-based
marker evidence, sysinfo process checks and Rustix native user IDs. It adds no
cleanup backend or application command execution.

- [Playwright browser management](https://playwright.dev/docs/browsers)
- [Puppeteer configuration](https://pptr.dev/guides/configuration)
- [Electron download cache](https://www.electronjs.org/docs/latest/tutorial/installation#cache)
- [Cypress binary cache](https://docs.cypress.io/app/references/advanced-installation#binary-cache)
- [pip caching](https://pip.pypa.io/en/stable/topics/caching/)
- [Go build and test caching](https://go.dev/cmd/go/#hdr-Build_and_test_caching)
- [sccache local cache](https://github.com/mozilla/sccache/blob/main/docs/Local.md)
- [PyTorch compile caching configuration](https://docs.pytorch.org/tutorials/recipes/torch_compile_caching_configuration_tutorial.html)
- [JetBrains directory meanings](https://www.jetbrains.com/help/idea/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html) and [index path ownership in PathManager](https://github.com/JetBrains/intellij-community/blob/master/platform/util/src/com/intellij/openapi/application/PathManager.java)
- [Conda package cache cleanup](https://docs.conda.io/projects/conda/en/stable/commands/clean.html)
- [Hugging Face cache structure](https://huggingface.co/docs/huggingface_hub/guides/manage-cache)
- [uv cache safety](https://docs.astral.sh/uv/concepts/cache/#cache-safety)

Tests use generated temporary directories, the real scanner, rule registry,
analysis and planning functions, injected process snapshots and a fake trash
backend. Platform-path fixtures are not Windows/Linux device tests. They do not
measure real GPU/IDE rebuilds, reclaimed space, or user demand. No real user caches
are scanned or moved during these tests.
