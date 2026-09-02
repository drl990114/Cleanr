import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const lockPath = path.join(root, "crates/rules/upstream-sources.json");
const builtinsRoot = path.join(root, "crates/rules/builtin-plugins");
const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));

if (lock.schema_version !== 1 || !Array.isArray(lock.sources)) {
  throw new Error("unsupported or invalid rule source lock");
}

const sourceIds = new Set();
for (const source of lock.sources) {
  for (const field of ["id", "repository", "revision", "license", "relation", "distribution"]) {
    if (typeof source[field] !== "string" || source[field].length === 0) {
      throw new Error(`source ${source.id ?? "<unknown>"} has invalid ${field}`);
    }
  }
  if (sourceIds.has(source.id)) throw new Error(`duplicate source id ${source.id}`);
  sourceIds.add(source.id);
  if (!/^https:\/\/github\.com\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(source.repository)) {
    throw new Error(`source ${source.id} repository is not a pinned GitHub project URL`);
  }
  if (!/^[0-9a-f]{40}$/.test(source.revision)) {
    throw new Error(`source ${source.id} revision must be a full 40-character commit`);
  }
  if (source.relation === "audited-against" && source.distribution !== "reference-only") {
    throw new Error(`source ${source.id} must remain reference-only`);
  }
  if (!["adapted", "audited-against", "independently-verified"].includes(source.relation)) {
    throw new Error(`source ${source.id} has an unsupported relation`);
  }
}

const tomlFiles = [];
function collectToml(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) collectToml(absolute);
    else if (entry.isFile() && entry.name.endsWith(".toml")) tomlFiles.push(absolute);
  }
}
collectToml(builtinsRoot);
const builtinText = tomlFiles.map((file) => fs.readFileSync(file, "utf8")).join("\n");

for (const source of lock.sources) {
  for (const expected of [source.id, source.repository, source.revision, source.license]) {
    if (!builtinText.includes(expected)) {
      throw new Error(`locked source ${source.id} is missing ${expected} from built-in metadata`);
    }
  }
}

const declaredIds = new Set(
  [...builtinText.matchAll(/^id = "([a-z0-9-]+)"$/gm)].map((match) => match[1]),
);
for (const id of ["kondo", "dusty", "puremac", "bleachbit", "winapp2"]) {
  if (!declaredIds.has(id)) throw new Error(`built-in source ${id} is not declared`);
}

process.stdout.write(`Validated ${lock.sources.length} pinned rule sources.\n`);
