#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { checkVersion, isMain, root } from "./release-lib.mjs";

function sections(text) {
  return [...text.matchAll(/^## (.+)\r?$/gm)].map((match) => ({
    title: match[1].trim().replace(/^\[(.*)\]$/, "$1"), start: match.index,
    bodyStart: match.index + match[0].length,
  }));
}

function section(text, title) {
  const headings = sections(text);
  const matches = headings.filter((heading) => heading.title === title);
  if (matches.length !== 1) throw new Error(`CHANGELOG.md needs exactly one ## ${title} section`);
  const heading = matches[0];
  const index = headings.indexOf(heading);
  const end = headings[index + 1]?.start ?? text.length;
  return { ...heading, end, body: text.slice(heading.bodyStart, end).trim() };
}

function requireNotes(body) {
  if (!/^- \S/m.test(body) || /\b(?:TODO|TBD)\b/.test(body)) {
    throw new Error("release notes need concrete change bullets, with no TODO/TBD placeholders");
  }
}

export function extractReleaseNotes(text, version) {
  checkVersion(version);
  const { body } = section(text, version);
  requireNotes(body);
  return `# Cleanr ${version}\n\n${body}\n`;
}

export function prepareReleaseNotes(text, version) {
  checkVersion(version);
  const unreleased = section(text, "Unreleased");
  if (sections(text).some((heading) => heading.title === version)) {
    if (unreleased.body) throw new Error(`CHANGELOG.md already contains ${version}; choose a new version for Unreleased changes`);
    extractReleaseNotes(text, version);
    return text;
  }
  requireNotes(unreleased.body);
  return text.slice(0, unreleased.start) + `## Unreleased\n\n## ${version}\n\n${unreleased.body}\n\n` + text.slice(unreleased.end);
}

if (isMain(import.meta.url)) {
  const [version, flag, output] = process.argv.slice(2);
  if (process.argv.length !== 5 || flag !== "--output") {
    throw new Error("usage: release-notes.mjs <version> --output <file>");
  }
  writeFileSync(output, extractReleaseNotes(readFileSync(join(root, "CHANGELOG.md"), "utf8"), version));
}
