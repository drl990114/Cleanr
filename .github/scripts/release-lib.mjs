import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { basename, delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = fileURLToPath(new URL("../..", import.meta.url));
export const platforms = readJson(join(root, "npm/platforms.json"));
export const smokeTargets = [
  "x86_64-unknown-linux-musl",
  "aarch64-apple-darwin",
  "x86_64-pc-windows-msvc",
];

export function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function checkVersion(version) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version ?? "")) {
    throw new Error("expected a stable version, for example 1.2.3");
  }
  return version;
}

export function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function integrity(path) {
  return `sha512-${createHash("sha512").update(readFileSync(path)).digest("base64")}`;
}

export function assetName(platform) {
  return `cleanr-${platform.target}${platform.binary.endsWith(".exe") ? ".exe" : ""}`;
}

// Invoke npm through Node, avoiding npm.cmd shell quoting on Windows.
export function npm(args, options = {}) {
  const nodeDir = dirname(process.execPath);
  const candidates = [
    process.env.npm_execpath,
    join(nodeDir, "node_modules/npm/bin/npm-cli.js"),
    join(nodeDir, "../lib/node_modules/npm/bin/npm-cli.js"),
    ...(process.env.PATH ?? "").split(delimiter).flatMap((directory) => [
      join(directory, "npm"),
      join(directory, "node_modules/npm/bin/npm-cli.js"),
    ]),
  ];
  const cli = candidates.find((candidate) => candidate && existsSync(candidate)
    && basename(realpathSync(candidate)) === "npm-cli.js");
  if (!cli) throw new Error("cannot locate npm-cli.js next to Node or on PATH");
  return execFileSync(process.execPath, [realpathSync(cli), ...args], {
    encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...options,
  });
}

export function readTarballs(directory, version) {
  checkVersion(version);
  const manifest = readJson(join(directory, "manifest.json"));
  const expected = [...platforms.map((platform) => platform.package), "cleanr-cli"];
  if (manifest.version !== version || manifest.schema_version !== 1
    || !Array.isArray(manifest.packages) || manifest.packages.length !== expected.length) {
    throw new Error("npm tarball manifest does not match the release");
  }
  for (const name of expected) {
    const matches = manifest.packages.filter((item) => item.name === name);
    if (matches.length !== 1) throw new Error(`expected exactly one tarball for ${name}`);
    const item = matches[0];
    if (!/^[a-zA-Z0-9_.-]+\.tgz$/.test(item.filename)
      || sha256(join(directory, item.filename)) !== item.sha256) {
      throw new Error(`npm tarball integrity mismatch: ${name}`);
    }
  }
  return manifest.packages;
}

export function isMain(url) {
  return process.argv[1] && resolve(process.argv[1]) === fileURLToPath(url);
}
