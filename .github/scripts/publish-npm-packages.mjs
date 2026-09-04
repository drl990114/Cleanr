#!/usr/bin/env node
// Publish the tarballs already exercised by the release smoke matrix.
import { join, resolve } from "node:path";
import { checkVersion, integrity, isMain, npm, readTarballs, root } from "./release-lib.mjs";
import { packPackages } from "./pack-npm-packages.mjs";

export function publishPackages(version, directory, { dryRun = false, runNpm = npm } = {}) {
  const packages = readTarballs(directory, version);
  // Enforce platform-first order even if the manifest was reordered.
  packages.sort((a, b) => Number(a.name === "cleanr-cli") - Number(b.name === "cleanr-cli"));
  for (const item of packages) {
    const tarball = resolve(directory, item.filename);
    if (!dryRun) {
      let publishedIntegrity;
      try {
        publishedIntegrity = JSON.parse(runNpm(["view", `${item.name}@${version}`, "dist.integrity", "--json"]));
      } catch (error) {
        const stderr = error.stderr?.toString() ?? "";
        if (!stderr.includes("E404")) throw error;
      }
      if (publishedIntegrity) {
        if (publishedIntegrity !== integrity(tarball)) {
          throw new Error(`${item.name}@${version} already exists with different bytes; use a new version`);
        }
        console.log(`verified existing ${item.name}@${version}`);
        continue;
      }
    }
    const args = ["publish", tarball, "--access", "public"];
    if (dryRun) args.push("--dry-run");
    runNpm(args, { stdio: "inherit" });
  }
}

if (isMain(import.meta.url)) {
  const [version, ...flags] = process.argv.slice(2);
  checkVersion(version);
  let directory = join(root, "npm-tarballs");
  let dryRun = false;
  let prepared = false;
  for (let index = 0; index < flags.length; index++) {
    if (flags[index] === "--dry-run") dryRun = true;
    else if (flags[index] === "--from-tarballs" && flags[index + 1]) {
      directory = resolve(flags[++index]);
      prepared = true;
    } else throw new Error("usage: publish-npm-packages.mjs <version> [--dry-run] [--from-tarballs <dir>]");
  }
  // Preserve standalone preparation. CI always supplies --from-tarballs,
  // preventing publishing from silently repacking previously tested bytes.
  if (!prepared) packPackages(version, join(root, "artifacts"), directory);
  publishPackages(version, directory, { dryRun });
}
