#!/usr/bin/env node
// Set the release version across workspace metadata files.
// Usage: node .github/scripts/set-release-version.mjs <version>

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { prepareReleaseNotes } from "./release-notes.mjs";
import { root } from "./release-lib.mjs";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("usage: set-release-version.mjs <version>");
  process.exit(1);
}

// Validate notes before touching version files; an old release cannot absorb
// new Unreleased changes under an already published version number.
const changelogPath = join(root, "CHANGELOG.md");
const changelog = prepareReleaseNotes(readFileSync(changelogPath, "utf8"), version);

function workspacePackages() {
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--no-deps", "--format-version", "1"],
      { cwd: root, encoding: "utf8" }
    )
  );
  const members = new Set(metadata.workspace_members);
  return metadata.packages.filter((pkg) => members.has(pkg.id));
}

function workspaceCargo() {
  const path = join(root, "Cargo.toml");
  let text = readFileSync(path, "utf8");
  text = text.replace(
    /^(\[workspace\.package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
    `$1"${version}"`
  );
  writeFileSync(path, text);
}

function crateCargos() {
  for (const pkg of workspacePackages()) {
    const path = pkg.manifest_path;
    let text = readFileSync(path, "utf8");
    // Update inline cleanr-* path dependency versions.
    text = text.replace(
      /(cleanr-[a-z0-9-]+\s*=\s*\{\s*version\s*=\s*)"[^"]+"/g,
      `$1"${version}"`
    );
    writeFileSync(path, text);
  }
}

function cargoLock() {
  execFileSync("cargo", ["update", "--workspace"], {
    cwd: root,
    stdio: "ignore",
  });
}

function npmPackage() {
  const path = join(root, "npm", "cleanr", "package.json");
  const pkg = JSON.parse(readFileSync(path, "utf8"));
  pkg.version = version;
  if (pkg.optionalDependencies) {
    for (const name of Object.keys(pkg.optionalDependencies)) {
      pkg.optionalDependencies[name] = version;
    }
  }
  writeFileSync(path, JSON.stringify(pkg, null, 2) + "\n");
}

function pluginFiles() {
  const output = [];
  for (const pluginsDir of [
    join(root, "plugins"),
    join(root, "crates", "rules", "builtin-plugins"),
  ]) {
    if (!existsSync(pluginsDir)) {
      continue;
    }
    for (const bundle of readdirSync(pluginsDir).sort()) {
      const bundleDir = join(pluginsDir, bundle);
      if (!statSync(bundleDir).isDirectory()) {
        continue;
      }
      const manifest = join(bundleDir, "plugin.toml");
      if (existsSync(manifest)) {
        output.push(manifest);
      }
      for (const child of ["rules", "locations", "locales"]) {
        const dir = join(bundleDir, child);
        if (!existsSync(dir)) {
          continue;
        }
        for (const name of readdirSync(dir).sort()) {
          const path = join(dir, name);
          if (statSync(path).isFile() && /\.(toml|ya?ml)$/.test(name)) {
            output.push(path);
          }
        }
      }
    }
  }
  return output;
}

function plugins() {
  for (const path of pluginFiles()) {
    let text = readFileSync(path, "utf8");
    text = text.replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`);
    writeFileSync(path, text);
  }
  execFileSync("node", [join(root, ".github/scripts/generate-plugin-index.mjs")], {
    cwd: root,
    stdio: "inherit",
  });
}

workspaceCargo();
crateCargos();
cargoLock();
npmPackage();
plugins();
writeFileSync(changelogPath, changelog);
console.log(`release: set version ${version}`);
