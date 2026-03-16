/**
 * Prebuild script — copies user-facing docs and ADRs from the canonical
 * `docs/` directory into Starlight's content collection directory.
 *
 * Only copies files intended for the public site (user guides + ADRs).
 * Plans, brainstorms, solutions, and other internal docs are excluded.
 */

import { cpSync, mkdirSync, rmSync, readdirSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const docsSource = resolve(__dirname, "../../docs");
const contentDest = resolve(__dirname, "../src/content/docs");

// Clean and recreate destination
rmSync(contentDest, { recursive: true, force: true });
mkdirSync(contentDest, { recursive: true });

// Copy top-level user docs (*.md files only, no subdirectories)
const topLevelFiles = readdirSync(docsSource).filter(
  (f) => f.endsWith(".md") && !f.startsWith(".")
);

for (const file of topLevelFiles) {
  cpSync(resolve(docsSource, file), resolve(contentDest, file));
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
  }

  console.log(`Copied ${adrFiles.length} ADRs`);
}
