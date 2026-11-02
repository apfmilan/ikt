// End-to-end passkey test against the REAL Rust API and the REAL built UI (apps/web/out).
// Uses Chromium's virtual WebAuthn authenticator (CTAP2, internal, user verification = Face ID
// equivalent). No broker is contacted; no order endpoint exists in this flow.
//
// Usage: TC_API=http://127.0.0.1:8081 SETUP_URL="http://localhost:8080/#token=..." node scripts/e2e/passkey-e2e.mjs
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const OUT = path.resolve("apps/web/out");
const API = new URL(process.env.TC_API ?? "http://127.0.0.1:8081");
const SETUP_URL = process.env.SETUP_URL;
if (!SETUP_URL) throw new Error("SETUP_URL required (from `tc setup-token`)");
const SHOT = process.env.SHOT ?? "";

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".svg": "image/svg+xml", ".webmanifest": "application/manifest+json", ".txt": "text/plain", ".woff2": "font/woff2" };
const server = http.createServer((req, res) => {
  if (req.url.startsWith("/api/")) {
    const p = http.request({ host: API.hostname, port: API.port, path: req.url, method: req.method, headers: req.headers }, (r) => {
      res.writeHead(r.statusCode ?? 502, r.headers);
      r.pipe(res);
    });
    p.on("error", () => { res.writeHead(502); res.end(); });
    req.pipe(p);
    return;
  }
  let f = path.join(OUT, decodeURIComponent(req.url.split("?")[0]));
  if (!f.startsWith(OUT)) { res.writeHead(403); return res.end(); }
  if (fs.existsSync(f) && fs.statSync(f).isDirectory()) f = path.join(f, "index.html");
  if (!fs.existsSync(f)) f = path.join(OUT, "index.html");
  res.writeHead(200, { "content-type": types[path.extname(f)] ?? "application/octet-stream" });
  fs.createReadStream(f).pipe(res);
});
await new Promise((r) => server.listen(8080, "127.0.0.1", r));

const results = [];
const check = (name, ok, detail = "") => { results.push({ name, ok, detail }); console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " — " + detail : ""}`); };
const browser = await chromium.launch();
try {
  const ctx = await browser.newContext({ viewport: { width: 393, height: 852 }, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  const pageErrors = [];
  page.on("pageerror", (e) => pageErrors.push(String(e.stack ?? e).split("\n").slice(0, 3).join(" | ")));
  const cdp = await ctx.newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: { protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
  });
  const api = (p, init = {}) => page.evaluate(async ([p, init]) => {
    const r = await fetch(p, { credentials: "same-origin", headers: { "content-type": "application/json" }, ...init });
    return { status: r.status, body: await r.json().catch(() => null) };
  }, [p, init]);

  // 1) First-passkey setup with the single-use token.
  await page.goto(SETUP_URL);
  await page.waitForURL(/\/setup/);
  await page.fill("#name", "E2E iPhone");
  await page.locator("form button[type=submit]").click();
  await page.getByText("Helyreállító kódok").waitFor({ timeout: 15000 });
  const codes = await page.locator(".font-mono span").allTextContents();
  check("setup registers passkey and shows 10 recovery codes", codes.length === 10, `${codes.length} codes`);
  const st1 = await api("/api/auth/status");
  check("session established after setup", st1.body?.authenticated === true);
  const creds = await cdp.send("WebAuthn.getCredentials", { authenticatorId });
  check("exactly one credential stored in the platform authenticator", creds.credentials.length === 1);

  // 2) Token is single use.
  const reuse = await api("/api/auth/setup/start", { method: "POST", body: JSON.stringify({ setup_token: new URL(SETUP_URL).hash.slice(7), device_name: "x" }) });
  check("setup token cannot be reused", reuse.status === 403, `status ${reuse.status}`);

  // 3) Logout, then passkey login through the real login page.
  await api("/api/auth/logout", { method: "POST" });
  const st2 = await api("/api/auth/status");
  check("logout ends session", st2.body?.authenticated === false);
  await page.goto("http://localhost:8080/login/");
  await page.locator("button", { hasText: /passkey|Face ID|Belépés/i }).first().click();
  await page.waitForURL((u) => !u.pathname.startsWith("/login"), { timeout: 15000 });
  const st3 = await api("/api/auth/status");
  check("passkey login creates session", st3.body?.authenticated === true);

  // 4) Protected endpoints and controls.
  const ov = await api("/api/overview");
  check("overview reachable with session", ov.status === 200, `mode=${ov.body?.mode}`);
  const freeze = await api("/api/control/kill", { method: "POST", body: JSON.stringify({ mode: "freeze" }) });
  check("emergency stop (freeze) needs only a session", freeze.status === 200);
  const release = await api("/api/control/kill", { method: "POST", body: JSON.stringify({ mode: "normal" }) });
  check("releasing the kill switch requires step-up", release.status === 403 && release.body?.error?.code === "stepup_required");

  // 5) Step-up passkey assertion bound to one action.
  const token = await page.evaluate(async () => {
    const b64 = (s) => { const pad = "=".repeat((4 - (s.length % 4)) % 4); const bin = atob((s + pad).replace(/-/g, "+").replace(/_/g, "/")); return Uint8Array.from(bin, (c) => c.charCodeAt(0)).buffer; };
    const enc = (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    const h = { "content-type": "application/json" };
    const s = await (await fetch("/api/auth/stepup/start", { method: "POST", headers: h, body: JSON.stringify({ action: "kill_release" }) })).json();
    const pk = s.options.publicKey;
    const cred = await navigator.credentials.get({ publicKey: { ...pk, challenge: b64(pk.challenge), allowCredentials: (pk.allowCredentials ?? []).map((c) => ({ ...c, id: b64(c.id) })) } });
    const r = cred.response;
    const body = { challenge_id: s.challenge_id, action: "kill_release", credential: { id: cred.id, rawId: enc(cred.rawId), type: cred.type, response: { authenticatorData: enc(r.authenticatorData), clientDataJSON: enc(r.clientDataJSON), signature: enc(r.signature), userHandle: r.userHandle ? enc(r.userHandle) : null }, extensions: {} } };
    const f = await (await fetch("/api/auth/stepup/finish", { method: "POST", headers: h, body: JSON.stringify(body) })).json();
    return f.stepup_token;
  });
  check("step-up assertion yields a token", typeof token === "string" && token.length > 20);
  const wrongAction = await api("/api/control/live", { method: "POST", headers: { "content-type": "application/json", "x-stepup-token": token }, body: JSON.stringify({ broker: "bitvavo", enabled: true }) });
  check("step-up token bound to its action (rejected for live_enable)", wrongAction.status === 403);
  const release2 = await api("/api/control/kill", { method: "POST", headers: { "content-type": "application/json", "x-stepup-token": token }, body: JSON.stringify({ mode: "normal" }) });
  check("kill switch release with step-up", release2.status === 200, `status ${release2.status}`);
  const replay = await api("/api/control/kill", { method: "POST", headers: { "content-type": "application/json", "x-stepup-token": token }, body: JSON.stringify({ mode: "normal" }) });
  check("step-up token is single-use (replay refused)", replay.status === 403);
  const pair = await api("/api/auth/pair", { method: "POST", body: "{}" });
  check("pairing a new device requires step-up", pair.status === 403 && pair.body?.error?.code === "stepup_required");
  const flat = await api("/api/control/kill", { method: "POST", body: JSON.stringify({ mode: "flatten" }) });
  check("flatten (sell everything) requires step-up", flat.status === 403 && flat.body?.error?.code === "stepup_required");
  const evil = await api("/api/push/subscribe", { method: "POST", body: JSON.stringify({ endpoint: "https://169.254.169.254/x", keys: { p256dh: "a", auth: "b" } }) });
  check("push endpoint restricted to browser push services", evil.status === 400);
  const live = await api("/api/control/live", { method: "POST", body: JSON.stringify({ broker: "bitvavo", enabled: true }) });
  check("enabling live trading without step-up is refused", live.status === 403);

  for (const route of ["/", "/positions/", "/bots/", "/timeline/", "/performance/", "/plan/", "/risk/", "/strategies/", "/trades/", "/settings/", "/onboarding/"]) {
    pageErrors.length = 0;
    await page.goto("http://localhost:8080" + route);
    await page.waitForTimeout(1200);
    const crashed = await page.getByText("This page couldn't load").count();
    check(`page ${route} renders with real API data`, crashed === 0 && pageErrors.length === 0, pageErrors.join(" ;; ").slice(0, 400));
  }
  if (SHOT) {
    await page.goto("http://localhost:8080/");
    await page.waitForTimeout(1500);
    await page.screenshot({ path: SHOT, fullPage: false });
  }
} finally {
  await browser.close();
  server.close();
}
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
