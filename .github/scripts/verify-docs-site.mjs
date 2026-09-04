#!/usr/bin/env node
import assert from "node:assert/strict";
import { setTimeout } from "node:timers/promises";
import { isMain } from "./release-lib.mjs";

export function pageAssets(html, pageUrl, baseUrl) {
  const assets = new Map();
  for (const match of html.matchAll(/<(script|link|img|video|source|meta)\b[^>]*>/gi)) {
    const tag = match[0];
    const kind = match[1].toLowerCase();
    // Docusaurus production output can omit quotes around safe attribute
    // values. Handle all HTML attribute forms without adding a DOM dependency.
    const attributes = Object.fromEntries([...tag.matchAll(/\b([a-zA-Z][\w:-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/g)]
      .map((attribute) => [attribute[1].toLowerCase(), attribute[2] ?? attribute[3] ?? attribute[4]]));
    if (kind === "link" && !attributes.rel?.toLowerCase().split(/\s+/).includes("stylesheet")) continue;
    let references;
    if (kind === "video") {
      references = [[attributes.poster, "image"], [attributes.src, "video"]];
    } else if (kind === "source") {
      // Ignore picture/audio source elements. Only video sources embedded in
      // these pages are checked; this does not crawl links or media catalogs.
      const videoSource = attributes.type ? /^video\//i.test(attributes.type)
        : attributes.src && /\.(mp4|webm|ogv|m4v|mov)$/i.test(new URL(attributes.src, pageUrl).pathname);
      references = videoSource ? [[attributes.src, "video"]] : [];
    } else if (kind === "meta") {
      references = attributes.property?.toLowerCase() === "og:image" ? [[attributes.content, "image"]] : [];
    } else {
      references = [[attributes[kind === "link" ? "href" : "src"], kind === "link" ? "css" : kind === "script" ? "js" : "image"]];
    }
    for (const [path, assetKind] of references) {
      if (!path) continue;
      const url = new URL(path.replaceAll("&amp;", "&"), pageUrl);
      if (url.origin !== baseUrl.origin) continue;
      assert.ok(url.pathname.startsWith(baseUrl.pathname), `asset escaped case-sensitive base path: ${url}`);
      assets.set(url.href, assetKind);
    }
  }
  assert.ok([...assets.values()].includes("css"), `page has no local stylesheet: ${pageUrl}`);
  assert.ok([...assets.values()].includes("js"), `page has no local JavaScript: ${pageUrl}`);
  return assets;
}

export async function verifyDocsSite(base, commit, { fetcher = fetch, attempts = 20, delay = 15_000 } = {}) {
  assert.match(commit, /^[0-9a-f]{40}$/);
  const baseUrl = new URL(base);
  assert.ok(baseUrl.pathname.endsWith("/"), "base URL must end with /");
  async function get(url) {
    const response = await fetcher(url, { signal: AbortSignal.timeout(15_000), headers: { "Cache-Control": "no-cache" } });
    assert.ok(response.ok, `HTTP ${response.status}: ${url}`);
    return response;
  }
  let lastError;
  for (let attempt = 0; attempt < attempts; attempt++) {
    try {
      const marker = new URL("deployment.json", baseUrl);
      marker.searchParams.set("commit", commit);
      assert.equal((await (await get(marker)).json()).commit, commit, "Pages is still serving an older deployment");
      const resources = new Map();
      for (const route of ["", "zh-Hans/", "docs/quick-start", "zh-Hans/docs/quick-start"]) {
        const url = new URL(route, baseUrl);
        const response = await get(url);
        assert.match(response.headers.get("content-type") ?? "", /text\/html/);
        const html = await response.text();
        assert.match(html, /Cleanr/);
        for (const asset of pageAssets(html, url, baseUrl)) resources.set(...asset);
      }
      for (const [url, kind] of resources) {
        const response = await get(url);
        const type = response.headers.get("content-type") ?? "";
        const expected = kind === "css" ? /text\/css/ : kind === "js" ? /(?:javascript|ecmascript)/
          : kind === "video" ? /^video\// : /^image\//;
        assert.match(type, expected, `wrong ${kind} MIME type (possible HTML fallback): ${url}`);
        assert.ok((await response.arrayBuffer()).byteLength > 0, `empty asset: ${url}`);
      }
      return { commit, pages: 4, assets: resources.size };
    } catch (error) {
      lastError = error;
      if (attempt + 1 < attempts) await setTimeout(delay);
    }
  }
  throw lastError;
}

if (isMain(import.meta.url)) {
  const [base, commit] = process.argv.slice(2);
  if (process.argv.length !== 4) throw new Error("usage: verify-docs-site.mjs <base-url> <commit>");
  console.log(JSON.stringify(await verifyDocsSite(base, commit)));
}
