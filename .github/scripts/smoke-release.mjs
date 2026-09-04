#!/usr/bin/env node
// Exercise downloaded release files, never a locally compiled development binary.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { assetName, checkVersion, isMain, npm, platforms, readJson, readTarballs, root, sha256 } from "./release-lib.mjs";

function normalized(path) {
  const value = resolve(path).replace(/^\\\\\?\\/, "");
  return process.platform === "win32" ? value.toLowerCase() : value;
}

export function validateAnalysis(report, sampleRoot) {
  assert.equal(report.schema_version, "cleanr.analysis.v1");
  assert.deepEqual(report.scan?.roots?.map(normalized), [normalized(sampleRoot)]);
  assert.equal(report.scan.integrity, "complete");
  assert.deepEqual(report.scan.issues, []);
  assert.ok(Array.isArray(report.candidates), "analyze must return candidates");
  assert.ok(report.candidates.some((candidate) => normalized(candidate.local_path) === normalized(join(sampleRoot, "node_modules"))),
    "analyze must discover the isolated dependency fixture");
}

function snapshot(directory) {
  return readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).map((entry) => {
    const path = join(directory, entry.name);
    const stat = statSync(path);
    return { name: entry.name, mtime: stat.mtimeMs, content: entry.isDirectory() ? snapshot(path) : sha256(path) };
  });
}

export function smokeRelease({ version, target, assetsDir, tarballsDir, reportPath, commit }) {
  checkVersion(version);
  assert.match(commit, /^[0-9a-f]{40}$/);
  const platform = platforms.find((entry) => entry.target === target);
  assert.ok(platform, `unknown target: ${target}`);
  assert.equal(platform.os, process.platform, "smoke must run on the actual target OS");
  assert.equal(platform.cpu, process.arch, "smoke must run on the actual target architecture");
  const packages = readTarballs(tarballsDir, version);
  const wrapper = packages.find((entry) => entry.name === "cleanr-cli");
  const native = packages.find((entry) => entry.name === platform.package);
  const asset = join(assetsDir, assetName(platform));
  const install = readJson(join(assetsDir, "install.json"));
  assert.equal(install.version, `v${version}`);
  assert.equal(sha256(asset), install.platforms[`${platform.os}-${platform.cpu}`].sha256);
  const temporary = mkdtempSync(join(tmpdir(), "cleanr-release-smoke-"));
  try {
    const project = join(temporary, "sample-project");
    const cache = join(project, "node_modules");
    mkdirSync(cache, { recursive: true });
    writeFileSync(join(project, "package.json"), '{"name":"cleanr-smoke-sample","private":true}\n');
    writeFileSync(join(cache, "fixture.bin"), Buffer.alloc(1024 * 1024 + 1, 0x61));
    const oldDate = new Date("2000-01-01T00:00:00Z");
    for (const path of [join(cache, "fixture.bin"), cache, join(project, "package.json"), project]) {
      utimesSync(path, oldDate, oldDate);
    }
    const sampleRoot = realpathSync(project);
    const before = snapshot(sampleRoot);
    const config = join(temporary, "config.toml");
    // Explicit configuration excludes installed plugins, language files and global roots.
    writeFileSync(config, '[scan]\nglobal_kinds = []\n[plugins]\ndirs = []\ntrusted = []\n[i18n]\ndirs = []\nlocale = "en-US"\n');
    const env = { ...process.env, CLEANR_NO_UPDATE_CHECK: "true" };
    function check(command, prefix = []) {
      const run = (args) => execFileSync(command, [...prefix, ...args], {
        cwd: temporary, env, encoding: "utf8", timeout: 30_000,
        maxBuffer: 4 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"],
      });
      assert.equal(run(["--version"]).trim(), `cleanr ${version}`);
      assert.match(run(["--help"]), /analyze/);
      validateAnalysis(JSON.parse(run(["--no-update-check", "--config", config, "analyze", sampleRoot])), sampleRoot);
      assert.deepEqual(snapshot(sampleRoot), before, "analyze must leave every sample file/directory unchanged");
      return ["version", "help", "analyze_read_only"];
    }
    const installedAsset = join(temporary, platform.binary);
    cpSync(asset, installedAsset);
    if (platform.os !== "win32") chmodSync(installedAsset, 0o755);
    const assetChecks = check(installedAsset);

    const installation = join(temporary, "npm-install");
    mkdirSync(installation);
    writeFileSync(join(installation, "package.json"), '{"name":"cleanr-smoke-install","private":true}\n');
    const userconfig = join(temporary, "npmrc");
    const globalconfig = join(temporary, "global-npmrc");
    writeFileSync(userconfig, "");
    writeFileSync(globalconfig, "");
    npm(["install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--package-lock=false",
      "--cache", join(temporary, "npm-cache"), "--userconfig", userconfig, "--globalconfig", globalconfig,
      "--registry", "http://127.0.0.1:9", resolve(tarballsDir, wrapper.filename), resolve(tarballsDir, native.filename)],
    { cwd: installation, timeout: 60_000 });
    assert.ok(existsSync(join(installation, "node_modules/.bin", platform.os === "win32" ? "cleanr.cmd" : "cleanr")),
      "npm must install its command shim");
    const launcher = join(installation, "node_modules/cleanr-cli/bin/cleanr.js");
    const npmChecks = check(process.execPath, [launcher]);
    // Reports contain no local paths or analysis data.
    const report = { schema_version: 1, version, commit, target,
      asset: { sha256: sha256(asset), checks: assetChecks },
      npm: { wrapper_sha256: wrapper.sha256, platform_sha256: native.sha256, checks: npmChecks },
    };
    mkdirSync(dirname(reportPath), { recursive: true });
    writeFileSync(reportPath, JSON.stringify(report, null, 2) + "\n");
    return report;
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

if (isMain(import.meta.url)) {
  const [version, target] = process.argv.slice(2);
  if (process.argv.length !== 4) throw new Error("usage: smoke-release.mjs <version> <target>");
  smokeRelease({ version, target, commit: process.env.GITHUB_SHA,
    assetsDir: join(root, "release-assets"), tarballsDir: join(root, "npm-tarballs"),
    reportPath: join(root, "smoke-reports", `${target}.json`),
  });
}
