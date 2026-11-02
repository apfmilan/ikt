// Renders apps/web/public/icons/icon.svg into the PNG sizes the PWA manifest and iOS need.
// Usage: node scripts/preview/make-icons.mjs   (uses the preinstalled Playwright Chromium)
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");
const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const iconsDir = join(root, "apps/web/public/icons");
const svg = readFileSync(join(iconsDir, "icon.svg"), "utf8");

// Maskable: full-bleed black background, glyph inside the 80 % safe zone.
const maskable = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" fill="#000"/>
<g transform="translate(76.8 76.8) scale(0.7)"><path d="M112 336 L208 240 L272 296 L400 160" fill="none" stroke="#a3e635" stroke-width="40" stroke-linecap="round" stroke-linejoin="round"/><circle cx="400" cy="160" r="30" fill="#a3e635"/></g></svg>`;
// Apple touch icon: iOS applies its own rounding, so render full-bleed.
const apple = svg.replace(/rx="112"/, 'rx="0"').replace(/<rect x="8"[^>]*\/>/, "");

const jobs = [
  { file: "icon-192.png", size: 192, src: svg },
  { file: "icon-512.png", size: 512, src: svg },
  { file: "icon-maskable-512.png", size: 512, src: maskable },
  { file: "apple-touch-icon.png", size: 180, src: apple },
];

const browser = await chromium.launch();
for (const j of jobs) {
  const page = await browser.newPage({ viewport: { width: j.size, height: j.size }, deviceScaleFactor: 1 });
  await page.setContent(
    `<html><body style="margin:0;background:transparent">${j.src.replace("<svg ", `<svg width="${j.size}" height="${j.size}" `)}</body></html>`,
  );
  await page.screenshot({ path: join(iconsDir, j.file), omitBackground: true, clip: { x: 0, y: 0, width: j.size, height: j.size } });
  await page.close();
  process.stdout.write(`icon ${j.file}\n`);
}
await browser.close();
