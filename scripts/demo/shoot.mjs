// Retakes the web view's screenshots from the demo `serve.sh` serves.
// Usage: PLAYWRIGHT=/path/to/playwright-core/index.mjs node scripts/demo/shoot.mjs OUT_DIR
import { mkdirSync } from "node:fs";

const { chromium } = await import(process.env.PLAYWRIGHT ?? "playwright");
const out = process.argv[2] ?? "docs/images";
mkdirSync(out, { recursive: true });

// name, path, width, height, mode, pixels to scroll
const shots = [
  ["web-view-desk-ask-1440-light", "/asks/1", 1440, 900, "light", 0],
  ["web-view-desk-ask-1440-dark", "/asks/1", 1440, 900, "dark", 0],
  ["web-view-ask-390-light", "/asks/1", 390, 844, "light", 0],
  ["web-view-answer-bar-390-dark", "/asks/1", 390, 844, "dark", 900],
  ["web-view-batch-390-light", "/", 390, 844, "light", 600],
  ["web-view-inbox-1440-light", "/inbox", 1440, 900, "light", 0],
  ["web-view-inbox-390-dark", "/inbox", 390, 844, "dark", 0],
  ["web-view-flows-1440-light", "/workflow", 1440, 900, "light", 0],
  ["web-view-flows-390-dark", "/workflow", 390, 844, "dark", 0],
  ["web-view-costs-1440-light", "/costs", 1440, 900, "light", 0],
  ["web-view-costs-390-dark", "/costs", 390, 844, "dark", 0],
];

const browser = await chromium.launch();
for (const [name, path, width, height, mode, scroll] of shots) {
  const phone = width < 600;
  const context = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: phone ? 2 : 1,
    isMobile: phone,
    hasTouch: phone,
    colorScheme: mode,
  });
  await context.addInitScript((mode) => {
    localStorage.setItem("priory.token", "demo-token");
    localStorage.setItem("priory.mode", mode);
  }, mode);
  const page = await context.newPage();
  await page.goto(`http://127.0.0.1:7391${path}`, { waitUntil: "load" });
  await page.waitForTimeout(2500);
  if (scroll) {
    await page.mouse.move(width / 2, height / 2);
    await page.mouse.wheel(0, scroll);
    await page.waitForTimeout(800);
  }
  await page.screenshot({ path: `${out}/${name}.png` });
  await context.close();
}
await browser.close();
