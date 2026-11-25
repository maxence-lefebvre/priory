// Runs the tests of components/, hooks/ and pages/. Node reads no JSX, so each test is bundled first,
// beside the packages it imports. The tests in this folder run with Verify's `node --test` line.
import { build } from "esbuild";
import { spawnSync } from "node:child_process";
import { globSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const cwd = fileURLToPath(new URL(".", import.meta.url));
const out = mkdtempSync(join(cwd, ".tests-"));
let status;
try {
  await build({
    absWorkingDir: cwd,
    entryPoints: globSync("{components,hooks,pages}/*.test.{js,jsx}", { cwd }),
    jsx: "automatic",
    bundle: true,
    packages: "external",
    platform: "node",
    format: "esm",
    outdir: out,
    outExtension: { ".js": ".mjs" },
    logLevel: "warning",
  });
  ({ status } = spawnSync(process.execPath, ["--test", ...globSync("**/*.mjs", { cwd: out })], { cwd: out, stdio: "inherit" }));
} finally {
  rmSync(out, { recursive: true, force: true });
}
process.exit(status ?? 1);
