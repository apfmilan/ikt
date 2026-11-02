"use client";

import { useEffect } from "react";

/** Registers the service worker (app-shell cache + push). Never caches API responses. */
export function ServiceWorkerRegister() {
  useEffect(() => {
    if (process.env.NODE_ENV !== "production") return;
    if (!("serviceWorker" in navigator)) return;
    navigator.serviceWorker.register("/sw.js", { scope: "/" }).catch((e: unknown) => console.warn("SW registration failed", e));
  }, []);
  return null;
}
