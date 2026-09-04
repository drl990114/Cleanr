import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { stageAssets } from "./generate-install-json.mjs";
import { assertReleaseReady } from "./check-release-gate.mjs";
import { packPackages } from "./pack-npm-packages.mjs";
import { publishPackages } from "./publish-npm-packages.mjs";
import { extractReleaseNotes, prepareReleaseNotes } from "./release-notes.mjs";
import { assetName, integrity, platforms, readJson, readTarballs, root, sha256, smokeTargets } from "./release-lib.mjs";
import { smokeRelease, validateAnalysis } from "./smoke-release.mjs";

const version = readJson(join(root, "npm/cleanr/package.json")).version;
const commit = "a".repeat(40);

function fixture(t) {
  const temporary = mkdtempSync(join(tmpdir(), "cleanr-release-test-"));
  t.after(() => rmSync(temporary, { recursive: true, force: true }));
  const artifactsDir = join(temporary, "artifacts");
  const tarballsDir = join(temporary, "tarballs");
  const assetsDir = join(temporary, "assets");
  mkdirSync(tarballsDir);
  for (const platform of platforms) {
    const directory = join(artifactsDir, `cleanr-${platform.target}`);
    mkdirSync(directory, { recursive: true });
    writeFileSync(join(directory, platform.binary), `fixture ${platform.target}`);
  }
  stageAssets(version, artifactsDir, assetsDir);
  const packages = [...platforms.map((platform) => platform.package), "cleanr-cli"].map((name, index) => {
    const filename = `fixture-${index}.tgz`;
    writeFileSync(join(tarballsDir, filename), name);
    return { name, filename, sha256: sha256(join(tarballsDir, filename)) };
  });
  writeFileSync(join(tarballsDir, "manifest.json"), JSON.stringify({ schema_version: 1, version, packages }));
  const needs = Object.fromEntries(["quality", "verify-release", "prepare-artifacts", "smoke"].map((key) => [key, { result: "success" }]));
  const reports = smokeTargets.map((target) => {
    const platform = platforms.find((item) => item.target === target);
    return { schema_version: 1, target, version, commit,
      asset: { sha256: sha256(join(assetsDir, assetName(platform))), checks: ["version", "help", "analyze_read_only"] },
      npm: { wrapper_sha256: packages.at(-1).sha256,
        platform_sha256: packages.find((item) => item.name === platform.package).sha256,
        checks: ["version", "help", "analyze_read_only"] },
    };
  });
  return { temporary, artifactsDir, tarballsDir, assetsDir, packages, needs, reports, version, commit };
}

test("release gate rejects failed, skipped, absent and stale checks/evidence", (t) => {
  const input = fixture(t);
  const evidence = assertReleaseReady(input);
  assert.equal(evidence.platforms.filter((platform) => platform.validation === "smoke-tested").length, 3);
  assert.equal(evidence.platforms.filter((platform) => platform.validation === "built-runtime-unverified").length, 4);
  for (const key of Object.keys(input.needs)) {
    for (const result of ["failure", "cancelled", "skipped", undefined]) {
      assert.throws(() => assertReleaseReady({ ...input, needs: { ...input.needs, [key]: { result } } }), /release requires/);
    }
  }
  assert.throws(() => assertReleaseReady({ ...input, reports: input.reports.slice(1) }), /all representative/);
  const stale = structuredClone(input.reports);
  stale[0].commit = "b".repeat(40);
  assert.throws(() => assertReleaseReady({ ...input, reports: stale }), /exact release commit/);
  const incomplete = structuredClone(input.reports);
  incomplete[2].npm.checks.pop();
  assert.throws(() => assertReleaseReady({ ...input, reports: incomplete }));
  writeFileSync(join(input.assetsDir, assetName(platforms[0])), "changed after smoke");
  assert.throws(() => assertReleaseReady(input));
});

test("install metadata and checksum file cover the exact staged bytes", (t) => {
  const { assetsDir } = fixture(t);
  const install = readJson(join(assetsDir, "install.json"));
  const sums = readFileSync(join(assetsDir, "SHA256SUMS"), "utf8").trim().split("\n");
  assert.equal(sums.length, 8);
  for (const line of sums) {
    const [digest, filename] = line.split("  ");
    assert.equal(digest, sha256(join(assetsDir, filename)));
  }
  for (const platform of platforms) {
    const entry = install.platforms[`${platform.os}-${platform.cpu}`];
    assert.equal(entry.sha256, sha256(join(assetsDir, assetName(platform))));
    assert.ok(entry.url.endsWith(`/v${version}/${assetName(platform)}`));
  }
});

test("publishing uses tested tarballs, orders wrapper last, and fails closed on mismatches", (t) => {
  const input = fixture(t);
  const calls = [];
  const missing = (args) => {
    calls.push(args);
    if (args[0] === "view") throw Object.assign(new Error("missing"), { stderr: "E404" });
  };
  publishPackages(version, input.tarballsDir, { runNpm: missing });
  const publications = calls.filter((args) => args[0] === "publish");
  assert.equal(publications.length, 8);
  assert.equal(publications.at(-1)[1], join(input.tarballsDir, input.packages.at(-1).filename));
  assert.throws(() => publishPackages(version, input.tarballsDir, { runNpm: () => JSON.stringify("sha512-other") }), /different bytes/);
  let publishes = 0;
  publishPackages(version, input.tarballsDir, { runNpm: (args) => {
    if (args[0] === "publish") publishes++;
    const item = input.packages.find((entry) => `${entry.name}@${version}` === args[1]);
    return JSON.stringify(integrity(join(input.tarballsDir, item.filename)));
  } });
  assert.equal(publishes, 0);
  writeFileSync(join(input.tarballsDir, input.packages[0].filename), "tampered");
  assert.throws(() => readTarballs(input.tarballsDir, version), /integrity mismatch/);
  assert.throws(() => publishPackages(version, input.tarballsDir, { runNpm: () => assert.fail("must fail before contacting npm") }), /integrity mismatch/);
});

test("release notes promote Unreleased once and reject empty, old-version and missing notes", () => {
  const changelog = "# Changelog\n\n## Unreleased\n\n- New behavior.\n\n### 简体中文\n\n- 新功能。\n\n## 0.1.0\n\n- Original.\n";
  const prepared = prepareReleaseNotes(changelog, "0.2.0");
  assert.match(prepared, /## Unreleased\n\n## 0.2.0/);
  assert.equal(prepareReleaseNotes(prepared, "0.2.0"), prepared);
  const notes = extractReleaseNotes(prepared, "0.2.0");
  assert.match(notes, /New behavior/);
  assert.match(notes, /新功能/);
  assert.doesNotMatch(notes, /Unreleased|Original/);
  assert.throws(() => extractReleaseNotes(changelog, "0.2.0"), /exactly one/);
  assert.throws(() => prepareReleaseNotes(changelog, "0.1.0"), /choose a new version/);
  assert.throws(() => prepareReleaseNotes("## Unreleased\n\nTBD\n", "0.2.0"), /concrete change bullets/);
});

function analysisFixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "cleanr-analysis-path-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const path = join(directory, "sample-project");
  const other = join(directory, "other-project");
  mkdirSync(join(path, "node_modules"), { recursive: true });
  mkdirSync(join(other, "node_modules"), { recursive: true });
  const report = { schema_version: "cleanr.analysis.v1", scan: { roots: [path], integrity: "complete", issues: [] },
    candidates: [{ local_path: join(path, "node_modules") }] };
  return { directory, path, other, report };
}

test("analysis validation rejects wrong roots, partial scans, and missing candidate evidence", (t) => {
  const { path, other, report } = analysisFixture(t);
  validateAnalysis(report, path);
  assert.throws(() => validateAnalysis({ ...report, candidates: [] }, path));
  assert.throws(() => validateAnalysis({ ...report, scan: { ...report.scan, integrity: "partial" } }, path));
  assert.throws(() => validateAnalysis(report, other));
  assert.throws(() => validateAnalysis({ ...report, candidates: [{ local_path: join(other, "node_modules") }] }, path));
});

test("analysis roots and candidates accept filesystem aliases without accepting a different directory", (t) => {
  const { directory, path, other, report } = analysisFixture(t);
  const alias = join(directory, "sample-alias");
  symlinkSync(path, alias, process.platform === "win32" ? "junction" : "dir");
  validateAnalysis(report, alias);
  validateAnalysis({ ...report, scan: { ...report.scan, roots: [alias] },
    candidates: [{ local_path: join(alias, "node_modules") }] }, path);
  assert.throws(() => validateAnalysis(report, other));
});

test("Windows temporary 8.3 paths match canonical report roots and candidates", { skip: process.platform !== "win32" }, (t) => {
  const { path, report } = analysisFixture(t);
  const canonical = realpathSync.native(path);
  // The hosted Windows runner exposes TEMP through RUNNER~1, as in the
  // release failure. Hosts without a short-name TEMP still run the alias test.
  if (!/~\d(?:\\|$)/i.test(path)) {
    t.skip("this Windows host does not expose an 8.3 TEMP path");
    return;
  }
  assert.notEqual(path.toLowerCase(), canonical.toLowerCase());
  validateAnalysis({ ...report, scan: { ...report.scan, roots: [canonical] },
    candidates: [{ local_path: join(canonical, "node_modules") }] }, path);
});

test("real npm packing and offline installation exercise the launcher and isolated smoke harness", { skip: process.platform === "win32" ? "Node-script fixture is not a Windows native executable; native Windows smoke runs in Release" : false }, (t) => {
  const input = fixture(t);
  // Unit-test executable fixture only; this is not release-binary acceptance.
  const executable = `#!/usr/bin/env node\nconst a=process.argv.slice(2);\nif(a[0]==="--version")console.log("cleanr ${version}");\nelse if(a[0]==="--help")console.log("Usage: cleanr analyze");\nelse if(a.includes("analyze")){const p=a.at(-1);console.log(JSON.stringify({schema_version:"cleanr.analysis.v1",scan:{roots:[p],integrity:"complete",issues:[]},candidates:[{local_path:require("node:path").join(p,"node_modules")}]}));}\nelse process.exit(3);\n`;
  for (const platform of platforms) {
    const path = join(input.artifactsDir, `cleanr-${platform.target}`, platform.binary);
    writeFileSync(path, executable);
    chmodSync(path, 0o755);
  }
  stageAssets(version, input.artifactsDir, input.assetsDir);
  packPackages(version, input.artifactsDir, input.tarballsDir);
  assert.equal(readTarballs(input.tarballsDir, version).length, 8);
  const platform = platforms.find((item) => item.os === process.platform && item.cpu === process.arch);
  const reportPath = join(input.temporary, "smoke.json");
  const report = smokeRelease({ ...input, target: platform.target, reportPath });
  assert.equal(report.target, platform.target);
  assert.equal(report.npm.checks.length, 3);
  assert.ok(existsSync(reportPath));
});
