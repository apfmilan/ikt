"use client";

// Authenticated WebSocket (server → client only) with reconnect + refetch-on-reconnect, and a
// small data hook that reloads REST resources when matching events arrive.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ApiError } from "./api";
import type { WsMessage, WsType } from "./types";

type Listener = (msg: WsMessage) => void;

interface LiveCtx {
  connected: boolean;
  /** Increments after every successful (re)connect; resources refetch when it changes. */
  epoch: number;
  subscribe: (fn: Listener) => () => void;
}

const Ctx = createContext<LiveCtx>({ connected: false, epoch: 0, subscribe: () => () => {} });

export function LiveProvider({ children, enabled }: { children: ReactNode; enabled: boolean }) {
  const [connected, setConnected] = useState(false);
  const [epoch, setEpoch] = useState(0);
  const listeners = useRef(new Set<Listener>());

  useEffect(() => {
    if (!enabled || typeof window === "undefined") return;
    let ws: WebSocket | null = null;
    let stopped = false;
    let attempt = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let ping: ReturnType<typeof setInterval> | undefined;

    const connect = () => {
      const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
      ws = new WebSocket(`${proto}//${window.location.host}/api/ws`);
      ws.onopen = () => {
        attempt = 0;
        setConnected(true);
        setEpoch((e) => e + 1);
        ping = setInterval(() => ws?.readyState === WebSocket.OPEN && ws.send(JSON.stringify({ type: "ping" })), 25_000);
      };
      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(String(ev.data)) as WsMessage;
          if (msg && typeof msg.type === "string") listeners.current.forEach((fn) => fn(msg));
        } catch {
          /* ignore malformed frames */
        }
      };
      ws.onclose = () => {
        setConnected(false);
        if (ping) clearInterval(ping);
        if (stopped) return;
        const delay = Math.min(30_000, 1000 * 2 ** attempt) * (0.75 + Math.random() * 0.5);
        attempt += 1;
        timer = setTimeout(connect, delay);
      };
      ws.onerror = () => ws?.close();
    };
    connect();

    const onVisible = () => {
      if (document.visibilityState === "visible" && ws?.readyState !== WebSocket.OPEN) {
        if (timer) clearTimeout(timer);
        attempt = 0;
        connect();
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      stopped = true;
      document.removeEventListener("visibilitychange", onVisible);
      if (timer) clearTimeout(timer);
      if (ping) clearInterval(ping);
      ws?.close();
    };
  }, [enabled]);

  const subscribe = useCallback((fn: Listener) => {
    listeners.current.add(fn);
    return () => {
      listeners.current.delete(fn);
    };
  }, []);

  const value = useMemo(() => ({ connected, epoch, subscribe }), [connected, epoch, subscribe]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useLive() {
  return useContext(Ctx);
}

export function useLiveEvent(types: WsType[], fn: Listener) {
  const { subscribe } = useLive();
  const ref = useRef(fn);
  useEffect(() => {
    ref.current = fn;
  });
  const key = types.join(",");
  useEffect(() => {
    const wanted = new Set(key.split(","));
    return subscribe((m) => {
      if (wanted.has(m.type)) ref.current(m);
    });
  }, [key, subscribe]);
}

export interface Resource<T> {
  data: T | undefined;
  error: ApiError | Error | undefined;
  loading: boolean;
  reload: () => void;
}

/** Fetch a resource; reload on reconnect and on the given live event types (debounced). */
export function useResource<T>(loader: (() => Promise<T>) | null, refreshOn: WsType[] = [], deps: unknown[] = []): Resource<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<ApiError | Error>();
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  const { epoch } = useLive();
  const loaderRef = useRef(loader);
  useEffect(() => {
    loaderRef.current = loader;
  });
  const debounce = useRef<ReturnType<typeof setTimeout>>(undefined);

  const reload = useCallback(() => setTick((t) => t + 1), []);

  useLiveEvent(refreshOn, () => {
    if (debounce.current) clearTimeout(debounce.current);
    debounce.current = setTimeout(reload, 300);
  });

  const depKey = JSON.stringify(deps);
  useEffect(() => {
    const fn = loaderRef.current;
    if (!fn) return;
    let alive = true;
    fn()
      .then((d) => {
        if (!alive) return;
        setData(d);
        setError(undefined);
      })
      .catch((e: unknown) => {
        if (!alive) return;
        setError(e instanceof Error ? e : new Error(String(e)));
      })
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [tick, epoch, depKey]);

  return { data, error, loading: loading && data === undefined, reload };
}
