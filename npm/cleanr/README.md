# cleanr-cli

AI-friendly disk cleanup. Safety comes first.

Cleanr is an AI-friendly, cross-platform disk cleanup tool. Review application
and browser caches, logs, temporary files, downloads, and development artifacts.
Candidates include reasons and risk notes; confirmed items are revalidated and
moved to system Trash with a local restore record. Choose folders or known cleanup
locations with the [scanning options](https://drl990114.github.io/Cleanr/docs/using-cleanr).

AI agents can use structured, read-only analysis to explain candidates and help
prepare a reviewable plan. You make the final cleanup decision. See the
[agent workflow](https://drl990114.github.io/Cleanr/docs/quick-start#ai-agent),
or review candidates directly in the terminal.

**Safety comes first.** Analysis is read-only and cleanup confirmation is on by
default. Agent execution checks the reviewed plan against a fresh scan; selected
paths and file state are rechecked before each move. Protected paths and
symbolic-link targets are rejected. See
[safety checks and recovery limits](https://drl990114.github.io/Cleanr/docs/safety-and-recovery).

Requires Node.js 18 or later. The package includes a launcher and selects a native
binary for your OS and CPU; see the [platform matrix](https://drl990114.github.io/Cleanr/docs/support-matrix).

```bash
npm install --global cleanr-cli
cleanr --version
cleanr /path/to/project
```

Use an existing project path. Press `s` to scan, `r` to review, and `q` to leave;
this first walkthrough does not require cleanup. Review defaults to a 90-day
observed modification threshold, so an empty list is possible. An AI agent
can use the read-only `cleanr analyze /path/to/folder` command for a chosen folder
or add explicit global categories for known cleanup locations.

Moving files to Trash usually does not immediately increase free disk space.
Restore is best-effort while the Trash item and local manifest still exist.
Cleanr does not upload reports, but an external agent may forward tool output
to a cloud model. The CLI approval flag is a caller assertion, not an OS sandbox.

Category filtering, cumulative filtered selection, `Shift+A` global selection,
and `cleanr.restore.v2` restore records apply to **0.15.0 and later**. Check
`cleanr --version` and read the [compatibility notes](https://drl990114.github.io/Cleanr/docs/support-matrix)
before rolling back to an older version.

- [Quick start, updating, rollback, and removal](https://drl990114.github.io/Cleanr/docs/quick-start)
- [Evidence and privacy](https://drl990114.github.io/Cleanr/docs/evidence-and-privacy)
- [Safety and recovery](https://drl990114.github.io/Cleanr/docs/safety-and-recovery)
- [Support](https://github.com/drl990114/Cleanr/blob/main/SUPPORT.md)
- [Changelog](https://github.com/drl990114/Cleanr/blob/main/CHANGELOG.md)
- [简体中文文档](https://drl990114.github.io/Cleanr/zh-Hans/)

Cleanr includes code adapted from [Byron/dua-cli](https://github.com/Byron/dua-cli),
an MIT-licensed disk usage analyzer by Sebastian Thiel and contributors.
