---
description: A real, read-only Cleanr terminal session using generated sample projects.
---

# First scan walkthrough

The homepage recording runs the published **Cleanr v0.14.0 on macOS Apple
Silicon**. Its 34-second terminal session uses generated sample projects. It
scans, opens Review, navigates, shows help, and quits. It never confirms cleanup
or runs restore.

## What happens in the recording

| Time | Action | What to look for |
| --- | --- | --- |
| 0–3 s | Open the sample project directory. | Start with one known directory. |
| 3–12 s | Press `s` to scan. | Two candidates total 24 MiB: a 16 MiB `node_modules` and an 8 MiB Rust `target`. Read the reason, age, and risk before selecting. |
| 12–18 s | Press `r` to open Review. | Review summarizes the selected candidates; opening it does not execute cleanup. |
| 18–26 s | Navigate with `j` and `k`. | Inspect both candidates and their details. |
| 26–30 s | Open help with `?`. | Check the available actions before continuing. |
| 30–34 s | Close help and quit. | The sample files remain unchanged. |

The fixtures also contain a 4 MiB `dist` file that is not matched in this run.
All fixtures are 120 days old. The recorder verifies file contents and
modification times after exiting. **24 MiB is the size of the candidates, not
disk space freed**. This demonstration does not validate the system Trash or
restore backend. See [recovery limits](./safety-and-recovery.md) and the
[platform evidence](./support-matrix.md).

Category filtering and `Shift+A` apply to **0.15.0 and later** and are not shown
in this v0.14.0 recording.

## Reproduce it

Use a trusted Cleanr binary, Python through `uv`, `ffmpeg`, and a monospaced font.
The recorder uses a POSIX terminal on macOS or Linux; it is not a Windows
recording tool. From the repository root:

```bash
uv run scripts/record-demo.py --binary /absolute/path/to/cleanr --output /tmp/cleanr-demo
```

On Linux, also pass `--font /absolute/path/to/monospace.ttf`. The script generates
its own temporary projects, sends only read-only navigation keys, and removes
those temporary fixtures on exit. It writes a PNG, MP4, original ANSI `.cast`,
plain-text scan, and version/hash metadata to the chosen output directory. Use
a new output directory to retain earlier recordings.

The homepage assets live under `docs/static/img/cleanr-scan.png` and
`docs/static/media/`. `cleanr-demo.json` records the binary SHA-256 and execution
environment. The source is `scripts/record-demo.py`; the terminal image is
rendered from the actual ANSI output using pyte and Pillow.

Ready to try your own read-only scan? Follow the [quick start](./quick-start.md).
