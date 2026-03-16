/**
 * Prebuild script — copies user-facing docs and ADRs from the canonical
 * `docs/` directory into Starlight's content collection directory.
 *
 * Only copies files intended for the public site (user guides + ADRs).
 * Plans, brainstorms, solutions, and other internal docs are excluded.
 */

import {
  cpSync,
  mkdirSync,
  rmSync,
  readdirSync,
  readFileSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const docsSource = resolve(__dirname, "../../docs");
const contentDest = resolve(__dirname, "../src/content/docs");

// Clean destination but preserve hand-crafted index.mdx
const indexPath = resolve(contentDest, "index.mdx");
let savedIndex = null;
if (existsSync(indexPath)) {
  savedIndex = readFileSync(indexPath, "utf-8");
}
rmSync(contentDest, { recursive: true, force: true });
mkdirSync(contentDest, { recursive: true });
if (savedIndex !== null) {
  writeFileSync(indexPath, savedIndex);
}

/**
 * Rewrite relative `.md` links to Starlight-compatible absolute paths.
 * e.g. `(configuration.md)` → `(/configuration/)`
 *      `(configuration.md#section)` → `(/configuration/#section)`
 *      `(adr/003-foo.md)` → `(/adr/003-foo/)`
 */
function rewriteLinks(filePath) {
  let content = readFileSync(filePath, "utf-8");
  const rewritten = content.replace(
    /\]\(([^)]+?)\.md(#[^)]+)?\)/g,
    "](/$1/$2)",
  );
  if (rewritten !== content) {
    writeFileSync(filePath, rewritten);
  }
}

// Copy top-level user docs (*.md files only, no subdirectories)
const topLevelFiles = readdirSync(docsSource).filter(
  (f) => f.endsWith(".md") && !f.startsWith(".")
);

for (const file of topLevelFiles) {
  cpSync(resolve(docsSource, file), resolve(contentDest, file));
  rewriteLinks(resolve(contentDest, file));
}

console.log(`Copied ${topLevelFiles.length} user docs`);

// Copy ADR directory
const adrSource = resolve(docsSource, "adr");
const adrDest = resolve(contentDest, "adr");

if (existsSync(adrSource)) {
  mkdirSync(adrDest, { recursive: true });
  const adrFiles = readdirSync(adrSource).filter((f) => f.endsWith(".md"));

  for (const file of adrFiles) {
    cpSync(resolve(adrSource, file), resolve(adrDest, file));
    rewriteLinks(resolve(adrDest, file));
  }

  console.log(`Copied ${adrFiles.length} ADRs`);
}
