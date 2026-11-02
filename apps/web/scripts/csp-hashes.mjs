// Post-build: collect SHA-256 hashes of every inline <script> in the static export so the
// reverse proxy can send a strict CSP without 'unsafe-inline' for scripts.
// Output: out/_csp/script-hashes.txt (one 'sha256-…' per line) and out/_csp/header.txt
// (a complete Content-Security-Policy value for Caddy).
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const OUT = new URL("../out/", import.meta.url).pathname;

function walk(dir, acc = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, acc);
    else if (p.endsWith(".html")) acc.push(p);
  }
  return acc;
}

let files;
try {
  files = walk(OUT);
} catch {
  console.warn("csp-hashes: no out/ directory (dev build?) — skipped");
  process.exit(0);
}

const hashes = new Set();
const re = /<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/g;
for (const f of files) {
  const html = readFileSync(f, "utf8");
  for (const m of html.matchAll(re)) {
    const body = m[1] ?? "";
    if (!body) continue;
    hashes.add(`'sha256-${createHash("sha256").update(body, "utf8").digest("base64")}'`);
  }
}

const list = [...hashes].sort();
const header = [
  "default-src 'self'",
  `script-src 'self' ${list.join(" ")}`,
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "font-src 'self'",
  "connect-src 'self'",
  "worker-src 'self'",
  "manifest-src 'self'",
  "frame-ancestors 'none'",
  "base-uri 'none'",
  "form-action 'self'",
  "object-src 'none'",
].join("; ");

mkdirSync(join(OUT, "_csp"), { recursive: true });
writeFileSync(join(OUT, "_csp", "script-hashes.txt"), list.join("\n") + "\n");
writeFileSync(join(OUT, "_csp", "header.txt"), header + "\n");
process.stdout.write(`csp-hashes: ${files.length} pages, ${list.length} inline script hashes → out/_csp/header.txt\n`);
