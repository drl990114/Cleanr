import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = process.cwd();

function fail(message) {
  throw new Error(`architecture boundary violation: ${message}`);
}

function sourceFiles(relativeDirectory) {
  const directory = path.join(root, relativeDirectory);
  const files = [];
  const visit = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const absolute = path.join(current, entry.name);
      if (entry.isDirectory()) visit(absolute);
      else if (entry.isFile() && entry.name.endsWith(".rs")) files.push(absolute);
    }
  };
  visit(directory);
  return files;
}

const workflowOnlySymbols = [
  "build_analysis_report_with_scan_context",
  "build_cleanup_plan_from_analysis",
  "resolve_scan_roots_with_locations",
  "scan_resolved_paths_with_progress_cancellable_started_at",
];

for (const relativeDirectory of ["crates/cli/src", "crates/tui/src"]) {
  for (const file of sourceFiles(relativeDirectory)) {
    const source = fs.readFileSync(file, "utf8");
    for (const symbol of workflowOnlySymbols) {
      if (new RegExp(`\\b${symbol}\\b`).test(source)) {
        fail(`${path.relative(root, file)} bypasses cleanr-tasks via ${symbol}`);
      }
    }
  }
}

for (const file of sourceFiles("crates/tasks/src")) {
  const source = fs.readFileSync(file, "utf8");
  if (/\bpub\s+(?:async\s+)?fn\s+execute_cleanup_plan\b/.test(source)) {
    fail(`${path.relative(root, file)} exposes the raw cleanup executor publicly`);
  }
}

const requiredModules = [
  "crates/core/src/model.rs",
  "crates/core/src/evidence.rs",
  "crates/core/src/planning.rs",
  "crates/core/src/safety.rs",
  "crates/core/src/manifests.rs",
  "crates/fs/src/roots.rs",
  "crates/fs/src/scanner.rs",
  "crates/fs/src/budget.rs",
  "crates/fs/src/identity.rs",
  "crates/rules/src/schema.rs",
  "crates/rules/src/loader.rs",
  "crates/rules/src/registry.rs",
  "crates/rules/src/matcher.rs",
  "crates/tasks/src/workflow.rs",
  "crates/tasks/src/cleanup.rs",
  "crates/tasks/src/restore.rs",
  "crates/tasks/src/storage.rs",
  "crates/tasks/src/platform.rs",
];
for (const relative of requiredModules) {
  if (!fs.existsSync(path.join(root, relative))) fail(`${relative} is missing`);
}

const i18nManifest = fs.readFileSync(path.join(root, "crates/i18n/Cargo.toml"), "utf8");
for (const dependency of ["reqwest", "sha2"]) {
  if (new RegExp(`^${dependency}\\s*=`, "m").test(i18nManifest)) {
    fail(`cleanr-i18n owns the distribution-only dependency ${dependency}`);
  }
}

const planning = fs.readFileSync(path.join(root, "crates/core/src/planning.rs"), "utf8");
for (const builder of ["build_cleanup_plan", "build_cleanup_plan_with_policy"]) {
  const declaration = planning.indexOf(`pub fn ${builder}`);
  if (declaration < 0) fail(`legacy builder ${builder} is missing`);
  const attributes = planning.slice(Math.max(0, declaration - 500), declaration);
  if (!attributes.includes("#[deprecated(")) fail(`legacy builder ${builder} is not deprecated`);
}

process.stdout.write("Validated Cleanr architecture boundaries.\n");
