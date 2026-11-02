"use client";

import { useSyncExternalStore } from "react";

const noopSubscribe = () => () => {};

/** Read a browser-only value without a hydration mismatch (server snapshot = fallback). */
export function useClientValue<T extends string | number | boolean>(read: () => T, fallback: T): T {
  return useSyncExternalStore(noopSubscribe, read, () => fallback);
}

/** Current URL fragment parameters (e.g. #token=…), updated on hashchange. */
export function useHashParam(name: string): string {
  return useSyncExternalStore(
    (cb) => {
      window.addEventListener("hashchange", cb);
      return () => window.removeEventListener("hashchange", cb);
    },
    () => new URLSearchParams(window.location.hash.slice(1)).get(name) ?? "",
    () => "",
  );
}
