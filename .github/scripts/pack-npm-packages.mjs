#!/usr/bin/env node
// Pack once; smoke tests and npm publishing consume these exact tarballs.
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { checkVersion, isMain, npm, platforms, readJson, root, sha256 } from "./release-lib.mjs";

export function packPackages(version, artifactsDir, outputDir, sourceRoot = root) {
  checkVersion(version);
  const wrapper = readJson(join(sourceRoot, "npm/cleanr/package.json"));
  if (wrapper.name !== "cleanr-cli" || wrapper.version !== version
    || Object.keys(wrapper.optionalDependencies ?? {}).length !== platforms.length
    || platforms.some((platform) => wrapper.optionalDependencies[platform.package] !== version)) {
    throw new Error("npm wrapper version/platforms do not match the release");
  }
  const staging = mkdtempSync(join(tmpdir(), "cleanr-pack-"));
  mkdirSync(outputDir, { recursive: true });
  const packages = [];
  function pack(name, directory, platform) {
    const [metadata] = JSON.parse(npm(["pack", "--json", "--pack-destination", resolve(outputDir)], { cwd: directory }));
    if (metadata.name !== name || metadata.version !== version) {
      throw new Error(`npm pack returned unexpected package: ${name}`);
    }
    const executable = metadata.files.find((file) => file.path === `bin/${platform?.binary ?? "cleanr.js"}`);
    if (!executable || (platform?.os !== "win32" && (Number(executable.mode) & 0o777) !== 0o755)) {
      throw new Error(`npm package is missing its executable or mode 755: ${name}`);
    }
    packages.push({ name, filename: metadata.filename, sha256: sha256(join(outputDir, metadata.filename)) });
  }
  try {
    for (const platform of platforms) {
      const directory = join(staging, platform.target);
      mkdirSync(join(directory, "bin"), { recursive: true });
      const binary = join(directory, "bin", platform.binary);
      cpSync(join(artifactsDir, `cleanr-${platform.target}`, platform.binary), binary);
      if (platform.os !== "win32") chmodSync(binary, 0o755);
      cpSync(join(sourceRoot, "LICENSE"), join(directory, "LICENSE"));
      writeFileSync(join(directory, "package.json"), JSON.stringify({
        name: platform.package, version, description: `Cleanr binary for ${platform.os}-${platform.cpu}`,
        license: "MIT", os: [platform.os], cpu: [platform.cpu], files: ["bin", "LICENSE"],
        repository: wrapper.repository,
      }, null, 2) + "\n");
      pack(platform.package, directory, platform);
    }
    const wrapperDirectory = join(staging, "wrapper");
    mkdirSync(wrapperDirectory);
    cpSync(join(sourceRoot, "npm/cleanr/package.json"), join(wrapperDirectory, "package.json"));
    cpSync(join(sourceRoot, "npm/cleanr/bin"), join(wrapperDirectory, "bin"), { recursive: true });
    const readme = join(sourceRoot, "npm/cleanr/README.md");
    if (existsSync(readme)) cpSync(readme, join(wrapperDirectory, "README.md"));
    cpSync(join(sourceRoot, "LICENSE"), join(wrapperDirectory, "LICENSE"));
    chmodSync(join(wrapperDirectory, "bin/cleanr.js"), 0o755);
    pack(wrapper.name, wrapperDirectory);
    writeFileSync(join(outputDir, "manifest.json"), JSON.stringify({ schema_version: 1, version, packages }, null, 2) + "\n");
    return packages;
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

if (isMain(import.meta.url)) {
  if (process.argv.length !== 3) throw new Error("usage: pack-npm-packages.mjs <version>");
  packPackages(process.argv[2], join(root, "artifacts"), join(root, "npm-tarballs"));
}
