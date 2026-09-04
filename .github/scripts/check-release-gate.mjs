#!/usr/bin/env node
import assert from "node:assert/strict";
import { appendFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { assetName, checkVersion, isMain, platforms, readJson, readTarballs, root, sha256, smokeTargets } from "./release-lib.mjs";

export function assertReleaseReady({ needs, version, commit, reports, assetsDir, tarballsDir }) {
  checkVersion(version);
  assert.match(commit, /^[0-9a-f]{40}$/);
  for (const required of ["quality", "verify-release", "prepare-artifacts", "smoke"]) {
    assert.equal(needs[required]?.result, "success", `release requires successful ${required} on this workflow's commit`);
  }
  const tarballs = readTarballs(tarballsDir, version);
  const wrapper = tarballs.find((item) => item.name === "cleanr-cli");
  assert.equal(reports.length, smokeTargets.length, "all representative platforms require smoke evidence");
  for (const target of smokeTargets) {
    const matches = reports.filter((report) => report.target === target);
    assert.equal(matches.length, 1, `missing or duplicate smoke report for ${target}`);
    const report = matches[0];
    const platform = platforms.find((item) => item.target === target);
    assert.equal(report.schema_version, 1);
    assert.equal(report.version, version);
    assert.equal(report.commit, commit, "smoke evidence must match the exact release commit");
    assert.equal(report.asset.sha256, sha256(join(assetsDir, assetName(platform))));
    assert.equal(report.npm.wrapper_sha256, wrapper.sha256);
    assert.equal(report.npm.platform_sha256, tarballs.find((item) => item.name === platform.package).sha256);
    for (const channel of [report.asset, report.npm]) {
      assert.deepEqual(channel.checks, ["version", "help", "analyze_read_only"]);
    }
  }
  return { schema_version: 1, version, commit,
    platforms: platforms.map((platform) => ({ target: platform.target,
      validation: smokeTargets.includes(platform.target) ? "smoke-tested" : "built-runtime-unverified",
    })),
    smoke: reports,
    limits: "Installation, version, help and isolated read-only analysis only; TUI interaction, trash and restore were not exercised.",
  };
}

if (isMain(import.meta.url)) {
  if (process.argv.length !== 3) throw new Error("usage: check-release-gate.mjs <version>");
  const evidence = assertReleaseReady({ needs: JSON.parse(process.env.RELEASE_NEEDS), version: process.argv[2],
    commit: process.env.GITHUB_SHA, reports: smokeTargets.map((target) => readJson(join(root, "smoke-reports", `${target}.json`))),
    assetsDir: join(root, "release-assets"), tarballsDir: join(root, "npm-tarballs"),
  });
  const evidencePath = join(root, "release-assets/release-evidence.json");
  writeFileSync(evidencePath, JSON.stringify(evidence, null, 2) + "\n");
  appendFileSync(join(root, "release-assets/SHA256SUMS"), `${sha256(evidencePath)}  release-evidence.json\n`);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `Release ${evidence.version} at ${evidence.commit}\n\n`
      + evidence.platforms.map((platform) => `- ${platform.target}: ${platform.validation}\n`).join("")
      + `\n${evidence.limits}\n`);
  }
}
