// Renders ONE design-preview PNG of the real UI (apps/web static export) at iPhone 15 Pro
// and desktop sizes, composited side by side → docs/preview/ui-preview.png.
//
// The /api/* responses come from scripts/preview/fixtures.json — clearly illustrative values
// that exist only here, never in the app bundle. The PNG carries a caption saying so.
//
// Usage: (cd apps/web && npm run build) && node scripts/preview/screenshot.mjs
import { createReadStream, existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const OUT = join(root, "apps/web/out");
const DEST = join(root, "docs/preview/ui-preview.png");
const raw = JSON.parse(readFileSync(join(here, "fixtures.json"), "utf8"));
// Shift every timestamp so the fixture "now" (_anchor) equals the render time.
const shift = Date.now() - new Date(raw._anchor).getTime();
const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/;
const reTime = (v) =>
  typeof v === "string" && ISO.test(v)
    ? new Date(new Date(v).getTime() + shift).toISOString()
    : Array.isArray(v)
      ? v.map(reTime)
      : v && typeof v === "object"
        ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, reTime(x)]))
        : v;
const fixtures = reTime(raw);

if (!existsSync(join(OUT, "index.html"))) {
  console.error("apps/web/out missing — run `npm run build` in apps/web first.");
  process.exit(1);
}

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".txt": "text/plain",
  ".woff2": "font/woff2",
};

// Minimal static server for the export (directory → index.html).
const server = createServer((req, res) => {
  const url = new URL(req.url ?? "/", "http://localhost");
  let p = normalize(join(OUT, decodeURIComponent(url.pathname)));
  if (!p.startsWith(OUT)) {
    res.writeHead(403).end();
    return;
  }
  if (existsSync(p) && statSync(p).isDirectory()) p = join(p, "index.html");
  if (!existsSync(p)) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { "Content-Type": TYPES[extname(p)] ?? "application/octet-stream" });
  createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;

function apiResponse(pathname) {
  const key = pathname.replace(/^\/api\//, "").replace(/\/$/, "");
  if (key in fixtures) return fixtures[key];
  return null;
}

async function shoot(browser, { width, height, scale, mobile }) {
  const ctx = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: scale,
    isMobile: mobile,
    hasTouch: mobile,
    colorScheme: "dark",
    locale: "hu-HU",
    timezoneId: "Europe/Budapest",
    userAgent: mobile
      ? "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"
      : undefined,
  });
  const page = await ctx.newPage();
  await page.route("**/api/**", (route) => {
    const body = apiResponse(new URL(route.request().url()).pathname);
    if (body === null) return route.fulfill({ status: 404, contentType: "application/json", body: '{"error":{"code":"not_found","message":"preview"}}' });
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
  });
  // Accept the live WebSocket so the UI shows a connected state; no frames are sent.
  await page.routeWebSocket(/\/api\/ws/, () => {});
  await page.goto(`${base}/`, { waitUntil: "networkidle" });
  await page.waitForSelector("canvas", { timeout: 15000 });
  await page.waitForTimeout(900);
  const buf = await page.screenshot({ type: "png" });
  await ctx.close();
  return buf;
}

const browser = await chromium.launch();
try {
  const phone = await shoot(browser, { width: 393, height: 852, scale: 3, mobile: true });
  const desktop = await shoot(browser, { width: 1440, height: 900, scale: 2, mobile: false });

  const page = await browser.newPage({ viewport: { width: 1600, height: 920 }, deviceScaleFactor: 2 });
  const img = (b) => `data:image/png;base64,${b.toString("base64")}`;
  await page.setContent(`<!doctype html><html><head><style>
    html,body{margin:0;background:#000;font-family:-apple-system,"Segoe UI",Roboto,sans-serif;color:#fafafa}
    .wrap{position:relative;width:1600px;height:920px;overflow:hidden;
      background:radial-gradient(1200px 600px at 70% -10%, #10160a 0%, #000 60%)}
    .title{position:absolute;left:56px;top:40px;font-size:30px;font-weight:600;letter-spacing:-.02em}
    .sub{position:absolute;left:56px;top:84px;font-size:15px;color:#8a8a8a}
    .desk{position:absolute;left:56px;top:136px;width:1104px;border-radius:18px;overflow:hidden;
      border:1px solid #262626;box-shadow:0 40px 120px rgba(0,0,0,.8)}
    .desk .bar{height:30px;background:#0d0d0d;border-bottom:1px solid #1f1f1f;display:flex;align-items:center;gap:7px;padding-left:14px}
    .desk .bar i{width:10px;height:10px;border-radius:50%;background:#2a2a2a;display:block}
    .desk img{display:block;width:1104px}
    .phone{position:absolute;right:64px;top:110px;width:338px;height:733px;border-radius:56px;background:#0c0c0c;
      border:1px solid #2a2a2a;padding:10px;box-shadow:0 40px 120px rgba(0,0,0,.85)}
    .phone img{display:block;width:338px;height:733px;border-radius:46px}
    .cap{position:absolute;left:56px;bottom:26px;font-size:13px;color:#5c5c5c;letter-spacing:.01em}
    .cap b{color:#8a8a8a;font-weight:500}
  </style></head><body><div class="wrap">
    <div class="title">Trading Center</div>
    <div class="sub">Mobil (iPhone PWA) és asztali (Windows / böngésző) nézet — Áttekintés</div>
    <div class="desk"><div class="bar"><i></i><i></i><i></i></div><img src="${img(desktop)}"></div>
    <div class="phone"><img src="${img(phone)}"></div>
    <div class="cap"><b>Design preview — illustrative values, not account data.</b> Rendered from the real UI build (apps/web) with mocked API responses.</div>
  </div></body></html>`);
  await page.waitForTimeout(300);
  mkdirSync(dirname(DEST), { recursive: true });
  await page.screenshot({ path: DEST, type: "png" });
  process.stdout.write(`preview written: ${DEST}\n`);
} finally {
  await browser.close();
  server.close();
}
