#!/usr/bin/env node
// Stage the exact GitHub assets with per-platform SHA-256 and SHA256SUMS.
import { cpSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { assetName, checkVersion, isMain, platforms, root, sha256 } from "./release-lib.mjs";

export function stageAssets(version, artifactsDir, outputDir) {
  checkVersion(version);
  mkdirSync(outputDir, { recursive: true });
  const releaseUrl = `https://github.com/drl990114/cleanr/releases/tag/v${version}`;
  const downloadBase = `https://github.com/drl990114/cleanr/releases/download/v${version}`;
  const install = {
    version: `v${version}`, notes: `Cleanr v${version}`, pub_date: new Date().toISOString(),
    release_url: releaseUrl, platforms: {},
  };
  const names = [];
  for (const platform of platforms) {
    const filename = assetName(platform);
    cpSync(join(artifactsDir, `cleanr-${platform.target}`, platform.binary), join(outputDir, filename));
    install.platforms[`${platform.os}-${platform.cpu}`] = {
      url: `${downloadBase}/${filename}`, sha256: sha256(join(outputDir, filename)),
    };
    names.push(filename);
  }
  writeFileSync(join(outputDir, "install.json"), JSON.stringify(install, null, 2) + "\n");
  names.push("install.json");
  writeFileSync(join(outputDir, "SHA256SUMS"), names.sort().map((name) => `${sha256(join(outputDir, name))}  ${name}\n`).join(""));
  return install;
}

if (isMain(import.meta.url)) {
  if (process.argv.length !== 3) throw new Error("usage: generate-install-json.mjs <version>");
  stageAssets(process.argv[2], join(root, "artifacts"), join(root, "release-assets"));
}
