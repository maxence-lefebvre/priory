// Bundles frame.jsx, with React and the web view's components it imports, into dist/frame.js, the
// file frame.html and command.html load. It is committed, so the folder loads unpacked with no build.
// With --check, builds into memory and fails when dist/frame.js differs or the bundle is too big.
import { build } from "esbuild";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const here = (path) => fileURLToPath(new URL(path, import.meta.url));
const OUT = here("./dist/frame.js");
// The bundle once the port ended, 273,809 bytes, plus 10%.
const BUDGET = 295 * 1024;

const result = await build({
  entryPoints: [here("./frame.jsx")],
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
  console.error(`dist/frame.js is ${bundle.length} bytes, over the ${BUDGET} byte budget.`);
  process.exit(1);
}

if (process.argv.includes("--check")) {
  let kept = null;
  try {
    kept = readFileSync(OUT);
  } catch {}
  if (!kept || !Buffer.from(bundle).equals(kept)) {
    console.error("adapters/github/dist/frame.js is stale: run corepack pnpm github:build and commit it.");
    process.exit(1);
  }
  console.log(`dist/frame.js is up to date, ${bundle.length} bytes.`);
} else {
  mkdirSync(here("./dist"), { recursive: true });
  writeFileSync(OUT, bundle);
  console.log(`wrote dist/frame.js, ${bundle.length} bytes.`);
}
