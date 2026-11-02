"use client";

import {
  ArrowLeftRight,
  Bot,
  ChartLine,
  FlaskConical,
  LayoutGrid,
  ListTree,
  LoaderCircle,
  Menu,
  PiggyBank,
  Plug,
  Settings,
  ShieldCheck,
  Wallet,
  type LucideIcon,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { api, ApiError } from "@/lib/api";
import { LiveProvider, useLive, useResource, type Resource } from "@/lib/live";
import type { AuthStatus, Overview } from "@/lib/types";
import { cn } from "@/lib/utils";
import { EmergencyStop, ModeBanner } from "./emergency-stop";
import { ErrorState } from "./states";
import { Sheet } from "./ui/sheet";

interface NavItem {
  href: string;
  label: string;
  icon: LucideIcon;
}

const PRIMARY: NavItem[] = [
  { href: "/", label: "Áttekintés", icon: LayoutGrid },
  { href: "/positions/", label: "Pozíciók", icon: Wallet },
  { href: "/bots/", label: "Botok", icon: Bot },
  { href: "/timeline/", label: "Napló", icon: ListTree },
];

const SECONDARY: NavItem[] = [
  { href: "/trades/", label: "Kereskedések", icon: ArrowLeftRight },
  { href: "/performance/", label: "Teljesítmény", icon: ChartLine },
  { href: "/strategies/", label: "Stratégiák", icon: FlaskConical },
  { href: "/plan/", label: "Pénzügyi terv", icon: PiggyBank },
  { href: "/risk/", label: "Kockázat", icon: ShieldCheck },
  { href: "/onboarding/", label: "Fiókok és élesítés", icon: Plug },
  { href: "/settings/", label: "Beállítások", icon: Settings },
];

const PUBLIC_ROUTES = new Set(["/login", "/setup"]);

const norm = (p: string) => (p.length > 1 && p.endsWith("/") ? p.slice(0, -1) : p);

const OverviewCtx = createContext<Resource<Overview> | null>(null);

/** Shared /overview resource (mode, live flags, balances) used by the shell and pages. */
export function useOverview(): Resource<Overview> {
  const ctx = useContext(OverviewCtx);
  if (!ctx) throw new Error("useOverview outside AppShell");
  return ctx;
}

export function AppShell({ children }: { children: ReactNode }) {
  const pathname = norm(usePathname() ?? "/");
  const router = useRouter();
  const isPublic = PUBLIC_ROUTES.has(pathname);
  const [auth, setAuth] = useState<AuthStatus | null>(null);
  const [authError, setAuthError] = useState<unknown>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let alive = true;
    api
      .authStatus()
      .then((s) => {
        if (!alive) return;
        setAuth(s);
        setAuthError(null);
        if (isPublic) return;
        if (s.setup_required) router.replace(`/setup/${window.location.hash}`);
        else if (!s.authenticated) router.replace("/login/");
      })
      .catch((e: unknown) => alive && setAuthError(e));
    return () => {
      alive = false;
    };
  }, [pathname, isPublic, router, attempt]);

  if (isPublic) return <div className="min-h-dvh">{children}</div>;

  if (authError) {
    return (
      <div className="mx-auto flex min-h-dvh max-w-md items-center px-5">
        <div className="w-full">
          <ErrorState error={authError instanceof ApiError ? authError : new Error("A szerver nem érhető el.")} onRetry={() => setAttempt((a) => a + 1)} />
        </div>
      </div>
    );
  }

  if (!auth || !auth.authenticated) {
    return (
      <div className="flex min-h-dvh items-center justify-center text-muted" aria-busy="true">
        <LoaderCircle className="animate-spin" size={20} aria-label="Betöltés" />
      </div>
    );
  }

  return (
    <LiveProvider enabled>
      <ShellFrame pathname={pathname}>{children}</ShellFrame>
    </LiveProvider>
  );
}

function ShellFrame({ pathname, children }: { pathname: string; children: ReactNode }) {
  const overview = useResource(api.overview, ["overview", "mode", "position", "order"]);
  const [moreOpen, setMoreOpen] = useState(false);
  const { connected } = useLive();
  const active = (href: string) => norm(href) === pathname;

  return (
    <OverviewCtx.Provider value={overview}>
      <div className="min-h-dvh lg:grid lg:grid-cols-[248px_1fr]">
        {/* Desktop sidebar */}
        <aside className="sticky top-0 hidden h-dvh flex-col border-r border-line px-3 py-5 lg:flex">
          <Brand />
          <nav className="mt-6 flex-1 space-y-0.5" aria-label="Fő navigáció">
            {[...PRIMARY, ...SECONDARY].map((n, i) => (
              <div key={n.href}>
                {i === PRIMARY.length && <div className="my-3 border-t border-line" />}
                <SideLink item={n} active={active(n.href)} />
              </div>
            ))}
          </nav>
          <ConnectionPill connected={connected} />
        </aside>

        <div className="min-w-0">
          {/* Top bar */}
          <header className="pt-safe sticky top-0 z-30 border-b border-line/70 bg-black/80 backdrop-blur-xl">
            <div className="mx-auto flex h-14 max-w-6xl items-center gap-2 px-4 lg:px-8">
              <div className="lg:hidden">
                <Brand compact />
              </div>
              <div className="flex-1" />
              <span className="hidden lg:block">
                <ConnectionPill connected={connected} inline />
              </span>
              <EmergencyStop mode={overview.data?.mode} onChanged={overview.reload} />
              <button
                className="rounded-xl p-2.5 text-muted hover:bg-surface-2 hover:text-fg lg:hidden"
                onClick={() => setMoreOpen(true)}
                aria-label="További menük"
              >
                <Menu size={20} />
              </button>
            </div>
          </header>

          <ModeBanner mode={overview.data?.mode} live={overview.data?.live} />

          <main className="mx-auto max-w-6xl px-4 pb-[calc(6rem+var(--safe-bottom))] pt-5 lg:px-8 lg:pb-12 lg:pt-8">{children}</main>
        </div>
      </div>

      {/* Mobile bottom tabs */}
      <nav
        aria-label="Fő navigáció"
        className="pb-safe fixed inset-x-0 bottom-0 z-40 border-t border-line bg-black/85 backdrop-blur-xl lg:hidden"
      >
        <div className="mx-auto grid h-16 max-w-md grid-cols-4">
          {PRIMARY.map((n) => {
            const Icon = n.icon;
            const on = active(n.href);
            return (
              <Link
                key={n.href}
                href={n.href}
                aria-current={on ? "page" : undefined}
                className={cn("flex flex-col items-center justify-center gap-1 text-[10.5px] font-medium transition-colors", on ? "text-fg" : "text-subtle")}
              >
                <Icon size={21} strokeWidth={on ? 2.2 : 1.8} />
                {n.label}
              </Link>
            );
          })}
        </div>
      </nav>

      <Sheet open={moreOpen} onClose={() => setMoreOpen(false)} title="Menü">
        <div className="grid grid-cols-2 gap-2">
          {SECONDARY.map((n) => {
            const Icon = n.icon;
            return (
              <Link
                key={n.href}
                href={n.href}
                onClick={() => setMoreOpen(false)}
                className="flex items-center gap-3 rounded-2xl border border-line bg-surface p-3.5 text-sm font-medium hover:bg-surface-3"
              >
                <Icon size={18} className="text-muted" />
                {n.label}
              </Link>
            );
          })}
        </div>
      </Sheet>
    </OverviewCtx.Provider>
  );
}

function Brand({ compact = false }: { compact?: boolean }) {
  return (
    <Link href="/" className="flex items-center gap-2.5 px-2" aria-label="Trading Center">
      <svg width="26" height="26" viewBox="0 0 64 64" aria-hidden>
        <rect width="64" height="64" rx="16" fill="#0f0f0f" stroke="#262626" />
        <path d="M14 42 L26 30 L34 37 L50 20" fill="none" stroke="#a3e635" strokeWidth="5" strokeLinecap="round" strokeLinejoin="round" />
        <circle cx="50" cy="20" r="4" fill="#a3e635" />
      </svg>
      {!compact && <span className="text-[15px] font-semibold tracking-tight">Trading Center</span>}
      {compact && <span className="text-[15px] font-semibold tracking-tight">TC</span>}
    </Link>
  );
}

function SideLink({ item, active }: { item: NavItem; active: boolean }) {
  const Icon = item.icon;
  return (
    <Link
      href={item.href}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-10 items-center gap-3 rounded-xl px-3 text-[14px] transition-colors",
        active ? "bg-surface-3 font-medium text-fg" : "text-muted hover:bg-surface-2 hover:text-fg",
      )}
    >
      <Icon size={17} strokeWidth={active ? 2.2 : 1.8} />
      {item.label}
    </Link>
  );
}

function ConnectionPill({ connected, inline = false }: { connected: boolean; inline?: boolean }) {
  return (
    <div className={cn("flex items-center gap-2 text-[12px] text-muted", inline ? "mr-2" : "px-3")}>
      <span className={cn("h-2 w-2 rounded-full", connected ? "bg-gain animate-pulse-dot" : "bg-warn")} aria-hidden />
      {connected ? "Élő kapcsolat" : "Újracsatlakozás…"}
    </div>
  );
}
