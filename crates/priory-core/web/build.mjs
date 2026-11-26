// Bundles main.jsx, with the React pages, the legacy ones, the view models and the packages, into dist/app.js, the file serve.rs embeds.
// With --check, builds into memory and fails when dist/app.js differs or the bundle is too big.
import { build } from "esbuild";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const here = (path) => fileURLToPath(new URL(path, import.meta.url));
const OUT = here("./dist/app.js");
const BUDGET = 720 * 1024;

const result = await build({
  entryPoints: [here("./main.jsx")],
  jsx: "automatic",
  define: { "process.env.NODE_ENV": '"production"' },
  bundle: true,
  format: "esm",
  minify: true,
  legalComments: "eof",
  charset: "utf8",
  target: "es2022",
  write: false,
  logLevel: "warning",
});
const bundle = result.outputFiles[0].contents;

if (bundle.length > BUDGET) {
  console.error(`dist/app.js is ${bundle.length} bytes, over the ${BUDGET} byte budget.`);
  process.exit(1);
}

if (process.argv.includes("--check")) {
  let kept = null;
  try {
    kept = readFileSync(OUT);
  } catch {}
  if (!kept || !Buffer.from(bundle).equals(kept)) {
    console.error("dist/app.js is stale: run corepack pnpm web:build and commit it.");
    process.exit(1);
  }
  console.log(`dist/app.js is up to date, ${bundle.length} bytes.`);
} else {
  mkdirSync(here("./dist"), { recursive: true });
  writeFileSync(OUT, bundle);
  console.log(`wrote dist/app.js, ${bundle.length} bytes.`);
}
