import type { NextConfig } from "next";
import { PHASE_DEVELOPMENT_SERVER } from "next/constants";

const apiTarget = process.env.TC_API_URL ?? "http://127.0.0.1:8080";

// Production: static export served by Caddy on the same origin as /api (cookies are
// SameSite=Strict + __Host-, so the UI must be same-origin with the API).
// Development (`next dev`): /api is proxied to a locally running `tc-server --role all`.
export default function config(phase: string): NextConfig {
  const base: NextConfig = {
    trailingSlash: true,
    reactStrictMode: true,
    poweredByHeader: false,
    images: { unoptimized: true },
  };
  if (phase === PHASE_DEVELOPMENT_SERVER) {
    return {
      ...base,
      async rewrites() {
        return [{ source: "/api/:path*", destination: `${apiTarget}/api/:path*` }];
      },
    };
  }
  return { ...base, output: "export" };
}
