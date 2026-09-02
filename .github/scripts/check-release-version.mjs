#!/usr/bin/env node
// Verify that workspace metadata files match the expected release version.
// Usage: node .github/scripts/check-release-version.mjs <version>

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("usage: check-release-version.mjs <version>");
  process.exit(1);
}

const root = new URL("../..", import.meta.url).pathname;
let failed = false;

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

function fail(message) {
  console.error(`release: ${message}`);
  failed = true;
}

function assertEqual(label, actual, expected) {
  if (actual !== expected) {
    fail(`${label} expected ${expected}, got ${actual}`);
  }
}

function workspaceCargo() {
  const text = readFileSync(join(root, "Cargo.toml"), "utf8");
  const match = text.match(/^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
  assertEqual("Cargo.toml [workspace.package] version", match?.[1], version);
}

function crateCargos() {
  for (const pkg of workspacePackages()) {
    const text = readFileSync(pkg.manifest_path, "utf8");
    const inline = [...text.matchAll(/cleanr-[a-z0-9-]+\s*=\s*\{\s*version\s*=\s*"([^"]+)"/g)];
    for (const m of inline) {
      assertEqual(`${pkg.name}/Cargo.toml dependency version`, m[1], version);
    }
  }
}

function npmPackage() {
  const pkg = JSON.parse(readFileSync(join(root, "npm", "cleanr", "package.json"), "utf8"));
  const platforms = JSON.parse(readFileSync(join(root, "npm", "platforms.json"), "utf8"));
  const platformPackages = new Set(platforms.map((p) => p.package));
  const optionalPackages = new Set(Object.keys(pkg.optionalDependencies ?? {}));

  assertEqual("npm package name", pkg.name, "cleanr-cli");
  assertEqual("npm/cleanr/package.json version", pkg.version, version);
  if (pkg.optionalDependencies) {
    for (const [name, v] of Object.entries(pkg.optionalDependencies)) {
      assertEqual(`optionalDependency ${name}`, v, version);
    }
  }

  assertEqual(
    "npm platform package count",
    optionalPackages.size,
    platformPackages.size
  );
  for (const name of platformPackages) {
    if (!name.startsWith("@cleanr-cli/")) {
      fail(`npm platform package ${name} must use the @cleanr-cli scope`);
    }
    if (!optionalPackages.has(name)) {
      fail(`npm optionalDependencies is missing ${name}`);
    }
  }

  const launcher = readFileSync(join(root, "npm", "cleanr", "bin", "cleanr.js"), "utf8");
  for (const name of platformPackages) {
    if (!launcher.includes(`"${name}"`)) {
      fail(`npm launcher is missing ${name}`);
    }
  }
}

function cargoLock() {
  const text = readFileSync(join(root, "Cargo.lock"), "utf8");
  for (const { name } of workspacePackages()) {
    const re = new RegExp(`\\[\\[package\\]\\]\\nname\\s*=\\s*"${name}"\\nversion\\s*=\\s*"([^"]+)"`);
    const match = text.match(re);
    assertEqual(`Cargo.lock ${name} version`, match?.[1], version);
  }
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
    const text = readFileSync(path, "utf8");
    const match = text.match(/^version\s*=\s*"([^"]+)"/m);
    assertEqual(path.replace(`${root}/`, ""), match?.[1], version);
  }
  try {
    execFileSync("node", [join(root, ".github/scripts/generate-plugin-index.mjs"), "--check"], {
      cwd: root,
      stdio: "inherit",
    });
  } catch {
    failed = true;
  }
}

workspaceCargo();
crateCargos();
npmPackage();
cargoLock();
plugins();

if (failed) {
  process.exit(1);
}
console.log(`release: version ${version} is consistent`);
