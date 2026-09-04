import assert from "node:assert/strict";
import test from "node:test";
import { pageAssets, verifyDocsSite } from "./verify-docs-site.mjs";

const base = "https://example.invalid/Cleanr/";
const commit = "c".repeat(40);
const html = '<title>Cleanr</title><link rel="stylesheet" href="/Cleanr/assets/main.css"><script src="/Cleanr/assets/main.js"></script><img src="/Cleanr/img/logo.svg">';
const minifiedHtml = '<title>Cleanr</title><link rel=stylesheet href=/Cleanr/assets/main.css /><script src=/Cleanr/assets/main.js defer></script><img src=/Cleanr/img/logo.svg />';

test("resource discovery accepts Docusaurus minified unquoted attributes", () => {
  const expected = pageAssets(html, new URL(base), new URL(base));
  assert.deepEqual(pageAssets(minifiedHtml, new URL(base), new URL(base)), expected);
  assert.deepEqual(pageAssets(html.replaceAll('"', "'"), new URL(base), new URL(base)), expected);
});

test("deployed site check requires the intended commit and working EN/zh resources", async () => {
  const urls = [];
  const fetcher = async (input) => {
    const url = new URL(input);
    urls.push(url.href);
    if (url.pathname.endsWith("deployment.json")) return Response.json({ commit });
    if (url.pathname.endsWith(".css")) return new Response("body{}", { headers: { "Content-Type": "text/css" } });
    if (url.pathname.endsWith(".js")) return new Response("void 0", { headers: { "Content-Type": "application/javascript" } });
    if (url.pathname.endsWith(".svg")) return new Response("<svg/>", { headers: { "Content-Type": "image/svg+xml" } });
    return new Response(minifiedHtml, { headers: { "Content-Type": "text/html" } });
  };
  assert.deepEqual(await verifyDocsSite(base, commit, { fetcher, attempts: 1 }), { commit, pages: 4, assets: 3 });
  assert.ok(urls.includes(`${base}zh-Hans/docs/quick-start`));
  await assert.rejects(verifyDocsSite(base, "d".repeat(40), { fetcher, attempts: 1 }), /older deployment/);
  const missingAsset = async (url, options) => String(url).endsWith("main.js")
    ? new Response("Not found", { status: 404 }) : fetcher(url, options);
  await assert.rejects(verifyDocsSite(base, commit, { fetcher: missingAsset, attempts: 1 }), /HTTP 404/);
  const htmlFallback = async (url, options) => String(url).endsWith("main.css")
    ? new Response(html, { headers: { "Content-Type": "text/html" } }) : fetcher(url, options);
  await assert.rejects(verifyDocsSite(base, commit, { fetcher: htmlFallback, attempts: 1 }), /HTML fallback/);
});

test("resource discovery rejects wrong-case base URLs", () => {
  assert.throws(() => pageAssets(html.replace("/Cleanr/assets/main.css", "/cleanr/assets/main.css"), new URL(base), new URL(base)), /case-sensitive base path/);
});

test("minified homepage video, poster and social card must exist with the correct media MIME", async () => {
  const mediaHtml = minifiedHtml
    + '<video controls preload=none poster=/Cleanr/img/walkthrough.png><source src=/Cleanr/video/walkthrough.mp4 type=video/mp4 /></video>'
    + '<meta property=og:image content=https://example.invalid/Cleanr/img/cleanr-social-card.png />'
    + '<source src=/Cleanr/audio/unrelated.mp3 type=audio/mpeg /><source src=/Cleanr/img/unrelated.webp type=image/webp />'
    + '<meta property=og:url content=https://example.invalid/Cleanr/elsewhere /><a href=/Cleanr/uncrawled>Link</a>';
  const assets = pageAssets(mediaHtml, new URL(base), new URL(base));
  assert.equal(assets.get(`${base}video/walkthrough.mp4`), "video");
  assert.equal(assets.get(`${base}img/walkthrough.png`), "image");
  assert.equal(assets.get(`${base}img/cleanr-social-card.png`), "image");
  assert.equal(assets.size, 6);
  const requests = [];
  const fetcher = async (input) => {
    const url = new URL(input);
    requests.push(url.pathname);
    if (url.pathname.endsWith("deployment.json")) return Response.json({ commit });
    const type = url.pathname.endsWith(".css") ? "text/css"
      : url.pathname.endsWith(".js") ? "application/javascript"
        : url.pathname.endsWith(".svg") ? "image/svg+xml"
          : url.pathname.endsWith(".png") ? "image/png"
            : url.pathname.endsWith(".mp4") ? "video/mp4" : "text/html";
    return new Response(type === "text/html" ? mediaHtml : "fixture bytes", { headers: { "Content-Type": type } });
  };
  assert.deepEqual(await verifyDocsSite(base, commit, { fetcher, attempts: 1 }), { commit, pages: 4, assets: 6 });
  assert.ok(requests.includes("/Cleanr/video/walkthrough.mp4"));
  assert.ok(requests.includes("/Cleanr/img/cleanr-social-card.png"));
  assert.ok(requests.every((path) => !/unrelated|uncrawled|elsewhere/.test(path)));
  for (const filename of ["walkthrough.mp4", "walkthrough.png", "cleanr-social-card.png"]) {
    const missing = async (url) => String(url).endsWith(filename)
      ? new Response("Not found", { status: 404 }) : fetcher(url);
    await assert.rejects(verifyDocsSite(base, commit, { fetcher: missing, attempts: 1 }), /HTTP 404/);
    const htmlFallback = async (url) => String(url).endsWith(filename)
      ? new Response(mediaHtml, { headers: { "Content-Type": "text/html" } }) : fetcher(url);
    await assert.rejects(verifyDocsSite(base, commit, { fetcher: htmlFallback, attempts: 1 }), /HTML fallback/);
  }
});
