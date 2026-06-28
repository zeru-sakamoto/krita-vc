/**
 * Takes screenshots of the key UI states in krita-vc.
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

await page.goto(URL, { waitUntil: "networkidle" });

async function shot(name) {
  await page.screenshot({ path: `${OUT}/${name}.png`, animations: "disabled" });
  console.log(`  ✓ ${name}`);
}

// 1. Default: History view, artist mode on, inspector open
await shot("01-history-default");

// 2. Close inspector
await page.click('button[aria-label="Hide inspector"]');
await shot("02-history-no-inspector");

// 3. Open inspector again, toggle artist mode off
await page.click('button[aria-label="Show inspector"]');
await page.click('button:has-text("Artist view")');
await shot("03-history-technical-mode");

// 4. Toggle artist mode back on
await page.click('button:has-text("Artist view")');

// 5. Changes view
await page.click('button[aria-label="Changes"]');
await shot("04-changes");

// 6. Branches view
await page.click('button[aria-label="Branches"]');
await shot("05-branches");

// 7. Back to history
await page.click('button[aria-label="History"]');
await shot("06-history-final");

await browser.close();
console.log(`\nScreenshots saved to ./${OUT}/`);
