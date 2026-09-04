# cleanr-cli

Review developer caches. Choose what goes to Trash.

Cleanr helps you inspect project dependencies, build output, and package-manager
caches from a terminal. Candidates include reasons and risk notes; selected
items are revalidated and moved to system Trash with a local restore record.

Requires Node.js 18 or later. The package includes a launcher and selects a native
binary for your OS and CPU; see the [platform matrix](https://drl990114.github.io/Cleanr/docs/support-matrix).

```bash
npm install --global cleanr-cli
cleanr --version
cleanr /path/to/project
```

Use an existing project path. Press `s` to scan, `r` to review, and `q` to leave;
this first walkthrough does not require cleanup. Review defaults to a 90-day
observed modification threshold, so an empty list is possible. A coding agent
can instead use the read-only `cleanr analyze /path/to/project` command.

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
