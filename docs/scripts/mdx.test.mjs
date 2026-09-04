import assert from 'node:assert/strict';
import {readdirSync, readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import path from 'node:path';
import test from 'node:test';
import {fileURLToPath} from 'node:url';

const siteDir = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(import.meta.url);
// Reuse the pinned Docusaurus parser and site settings without building a site.
const coreRequire = createRequire(require.resolve('@docusaurus/core/package.json'));
const loaderRequire = createRequire(coreRequire.resolve('@docusaurus/mdx-loader'));
const {loadSiteConfig} = coreRequire('./lib/server/config.js');
const {compileToJSX} = loaderRequire('./utils.js');
const {DEFAULT_PARSE_FRONT_MATTER} = coreRequire('@docusaurus/utils');
const {siteConfig} = await loadSiteConfig({siteDir});
const options = {
  siteDir,
  staticDirs: siteConfig.staticDirectories.map((dir) => path.resolve(siteDir, dir)),
  markdownConfig: siteConfig.markdown,
  admonitions: true,
  removeContentTitle: true,
};
const files = ['docs', 'i18n'].flatMap((dir) =>
  readdirSync(path.join(siteDir, dir), {recursive: true})
    .filter((file) => /\.mdx?$/.test(file))
    .map((file) => path.join(dir, file)),
).sort();
assert.ok(files.length > 0, 'No documentation files found');

for (const relativePath of files) {
  test(`MDX syntax: ${relativePath}`, async () => {
    const filePath = path.join(siteDir, relativePath);
    const fileContent = readFileSync(filePath, 'utf8');
    const {frontMatter} = await siteConfig.markdown.parseFrontMatter({
      filePath,
      fileContent,
      defaultParseFrontMatter: DEFAULT_PARSE_FRONT_MATTER,
    });
    const result = await compileToJSX({
      filePath,
      fileContent,
      frontMatter,
      options,
      compilerName: 'client',
    });
    if (path.basename(filePath) === 'quick-start.md') {
      assert.match(result.content, /id: "ai-agent"/, 'Preserve the homepage AI agent anchor');
    }
  });
}
