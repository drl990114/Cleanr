# Continue with incomplete evidence

Read this file when `scan.integrity` is `partial` or a scan reports unreadable
locations. Extra OS permissions are optional, never a prerequisite for this
skill. Do not ask for Full Disk Access, `sudo`, ownership or permission changes,
or disabling system protections to continue. Discuss access changes only when
the user explicitly asks how to include a skipped location.

## Identify the cause

Inspect `scan.issues` and, when present, `scan.budget_exceeded`.
`permission-denied` identifies an access failure; `metadata-unavailable`,
`root-unavailable`, `traversal-error`, and `unknown` do not by themselves prove
that macOS privacy permission is missing. A nonempty budget ledger means a scan
limit was reached; keep that result read-only and do not try to build a plan
from it. Do not raise budgets or change persistent configuration automatically.

## Continue within the approved scope

- Explain the collected evidence and list unreadable or omitted locations as
  coverage gaps. Keep the original report partial; never call these locations
  empty, safe, fully scanned, or cleaned.
- For an analysis-only request, finish the review using the available evidence.
  For planning or cleanup, try one smaller-scope analysis when a useful subset
  can be isolated. Use unaffected approved roots or fewer already requested
  `--global-kind` values. Explicit child roots must stay within approved
  user-level locations and preserve the requested categories; omit `--global`
  when switching to explicit roots so it does not add the blocked locations
  again. Do not scan a broader ancestor to recover a candidate.
- Omit both unreadable locations and any chosen root that would traverse them.
  If an issue has no usable path, do not guess which subtree is complete.
  Scan roots themselves are protected: scanning a candidate directory as a
  root does not make that directory selectable.
- State the narrower scope and skipped locations, then continue without a new
  scope or permissions question. Preserve the agreed data-processing boundary,
  config, and inactivity policy. Do not edit the report, suppress error records,
  or invent an ignore-permissions flag.
- Use only a fresh complete analysis for automatic plan selections. Plan the
  exact same narrower scope and policy, retain the original coverage gaps in
  the human summary, and apply the normal final confirmation requirements.
  Read-only `plan`/`dry-run` inspection is also allowed for a partial report
  without budget exhaustion if the CLI permits it; default selection may be
  empty. Never force-select items to bypass incomplete evidence.
- If no useful subset remains, the retry is still partial, or plan generation
  refuses or selects nothing, summarize the available evidence and the
  limitation. Stop without repeatedly retrying or requesting more OS access.
  If the plan's fresh scan changes selected-item evidence or safety provenance,
  do not execute it without a new review and exact confirmation.

A complete narrower scan does not make the original global scan complete.
Unreadable Safari, CloudKit, or temporary locations can stay unexamined while
other approved locations are reviewed.
