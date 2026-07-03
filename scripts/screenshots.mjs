/**
 * Takes screenshots of the UI in a plain browser. There is no mock data — without the
 * Tauri backend the app renders its empty states (fresh profile = the welcome shell),
 * so this is a smoke test that the browser build boots cleanly, not a feature tour.
 * Real-repository screenshots require the desktop shell (`npm run tauri dev`).
 *
 * Requires the dev server to be running: npm run dev
 * Usage: node scripts/screenshots.mjs
 */

import { chromium } from "playwright";
import { mkdirSync } from "fs";

const OUT = "screenshots";
const URL = "http://localhost:1420";

mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => {
  if (m.type() === "error") errors.push(m.text());
});

await page.goto(URL, { waitUntil: "networkidle" });

// Fresh profile → no repositories → the welcome shell.
await page.screenshot({ path: `${OUT}/01-welcome.png`, animations: "disabled" });
console.log("  ✓ 01-welcome");

if (errors.length) {
  console.error("PAGE ERRORS:\n" + errors.join("\n"));
  process.exit(1);
}

await browser.close();
console.log(`\nScreenshots saved to ./${OUT}/ (no console/page errors)`);
