"use client";

import { Bitcoin, ChevronRight, Landmark, LineChart as LineChartIcon } from "lucide-react";
import Link from "next/link";
import { EquityChart } from "@/components/charts";
import { DecisionItem } from "@/components/decision";
import { Huf, Kind, Pct } from "@/components/money";
import { PageHeader, Section, Stat } from "@/components/page-header";
import { useOverview } from "@/components/shell";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge, Dot } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { ago, BROKER_LABEL, dateTime } from "@/lib/format";
import { useResource } from "@/lib/live";
import type { BotCard, BotId, BrokerCard } from "@/lib/types";
import { cn } from "@/lib/utils";

const BOT_ICON: Record<BotId, typeof Bitcoin> = { crypto: Bitcoin, stocks: LineChartIcon, invest: Landmark };
const BOT_STATUS: Record<BotCard["status"], { label: string; tone: "gain" | "neutral" | "warn" | "loss" }> = {
  active: { label: "Aktív", tone: "gain" },
  idle: { label: "Figyel", tone: "neutral" },
  paused: { label: "Szünetel", tone: "warn" },
  blocked: { label: "Blokkolva", tone: "warn" },
  error: { label: "Hiba", tone: "loss" },
};

export default function OverviewPage() {
  const { data, error, loading, reload } = useOverview();
  const decisions = useResource(() => api.decisions(4), ["decision"]);

  return (
    <div className="animate-fade-up">
      <PageHeader title="Áttekintés" subtitle={data ? `Frissítve ${ago(data.as_of)}` : undefined} />
      {error && !data && <ErrorState error={error} onRetry={reload} />}

      {/* Equity hero */}
      <Card className="overflow-hidden">
        <div className="p-5 pb-2">
          <div className="flex items-center justify-between">
            <span className="text-[13px] text-muted">Teljes tőke</span>
            <Kind kind="realized" />
          </div>
          {loading ? (
            <Skeleton className="mt-2 h-10 w-48" />
          ) : (
            <div className="mt-1 text-[40px] font-semibold leading-none tracking-[-0.03em] num">
              <Huf value={data?.realized.equity_huf} />
            </div>
          )}
          <div className="mt-5 grid grid-cols-2 gap-4 sm:grid-cols-4">
            <Stat label="Mai nettó eredmény">{data ? <Huf value={data.realized.net_pnl_today_huf} signed /> : "–"}</Stat>
            <Stat label="Összes nettó eredmény" hint="befizetések nélkül">
              {data ? <Huf value={data.realized.net_pnl_total_huf} signed /> : "–"}
            </Stat>
            <Stat label="Nettó befizetés">{data ? <Huf value={data.realized.net_deposits_huf} /> : "–"}</Stat>
            <Stat label="Visszaesés a csúcstól">
              {data ? <Pct value={`-${data.realized.drawdown_pct}`} /> : "–"}
            </Stat>
          </div>
        </div>
        <div className="px-1 pb-2">
          <EquityChart points={data?.equity_curve ?? []} />
        </div>
        <div className="flex items-center gap-4 border-t border-line px-5 py-3 text-[12px] text-muted">
          <span className="flex items-center gap-1.5">
            <span className="h-0.5 w-4 rounded bg-gain" /> Tőke
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-0 w-4 border-t border-dashed border-subtle" /> Befizetések
          </span>
          <span className="ml-auto">
            Díjak összesen <Huf value={data?.realized.fees_total_huf} />
          </span>
        </div>
      </Card>

      <Section title="Botok" action={<SeeAll href="/bots/" />}>
        {loading ? (
          <LoadingRows rows={3} />
        ) : data && data.bots.length > 0 ? (
          <div className="grid gap-3 sm:grid-cols-3">
            {data.bots.map((b) => (
              <BotTile key={b.id} bot={b} />
            ))}
          </div>
        ) : (
          <EmptyState title="Nincs futó bot" hint="A botok az élesítés után indulnak." />
        )}
      </Section>

      <Section title="Számlák">
        {loading ? (
          <LoadingRows rows={2} />
        ) : data && data.brokers.length > 0 ? (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {data.brokers.map((b) => (
              <BrokerTile key={b.broker} b={b} live={Boolean(data.live[b.broker])} />
            ))}
          </div>
        ) : (
          <EmptyState
            title="Nincs csatlakoztatott számla"
            hint="Add meg a Trading 212 és Bitvavo API-kulcsokat."
            action={
              <Link href="/onboarding/" className="mt-2 text-[13px] font-medium text-accent">
                Beállítás indítása
              </Link>
            }
          />
        )}
      </Section>

      <Section title="Legutóbbi döntések" action={<SeeAll href="/timeline/" />}>
        {decisions.loading ? (
          <LoadingRows rows={2} />
        ) : decisions.error ? (
          <ErrorState error={decisions.error} onRetry={decisions.reload} />
        ) : decisions.data && decisions.data.length > 0 ? (
          <Card className="divide-y divide-line">
            {decisions.data.map((d) => (
              <DecisionItem key={d.id} d={d} compact />
            ))}
          </Card>
        ) : (
          <EmptyState title="Még nincs döntés" hint="Az ügynökök minden lezárt gyertya után értékelnek; a tartózkodás is naplózásra kerül." />
        )}
      </Section>
    </div>
  );
}

function SeeAll({ href }: { href: string }) {
  return (
    <Link href={href} className="flex items-center gap-0.5 text-[13px] text-muted hover:text-fg">
      Mind <ChevronRight size={14} />
    </Link>
  );
}

function BotTile({ bot }: { bot: BotCard }) {
  const Icon = BOT_ICON[bot.id] ?? LineChartIcon;
  const st = BOT_STATUS[bot.status];
  return (
    <Link href="/bots/" className="group">
      <Card className="h-full p-4 transition-colors group-hover:bg-surface-2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-surface-3">
              <Icon size={18} className="text-fg" />
            </span>
            <span className="text-[15px] font-semibold">{bot.name}</span>
          </div>
          <Badge tone={st.tone}>
            <Dot className={cn(bot.status === "active" && "animate-pulse-dot")} />
            {st.label}
          </Badge>
        </div>
        <div className="mt-4 flex items-end justify-between">
          <div>
            <div className="text-[12px] text-muted">Ma</div>
            <Huf value={bot.today_pnl_huf} signed className="text-[20px] font-semibold tracking-tight" />
          </div>
          <div className="text-right text-[12px] text-muted">
            <div className="num">{bot.open_positions} nyitott</div>
            <div>{bot.last_decision_at ? ago(bot.last_decision_at) : "–"}</div>
          </div>
        </div>
        {bot.message && <p className="mt-3 line-clamp-2 text-[12.5px] leading-snug text-muted">{bot.message}</p>}
      </Card>
    </Link>
  );
}

function BrokerTile({ b, live }: { b: BrokerCard; live: boolean }) {
  const health = { ok: { t: "gain", l: "Rendben" }, degraded: { t: "warn", l: "Lassú" }, down: { t: "loss", l: "Nem elérhető" } } as const;
  const h = health[b.health];
  return (
    <Card className="p-4">
      <div className="flex items-center justify-between">
        <span className="text-[15px] font-semibold">{BROKER_LABEL[b.broker] ?? b.broker}</span>
        <div className="flex items-center gap-1.5">
          {b.broker === "revolut" ? <Badge>Csak jelzés</Badge> : live ? <Badge tone="accent">Éles</Badge> : <Badge tone="warn">Nem élesített</Badge>}
          <Badge tone={h.t}>
            <Dot /> {h.l}
          </Badge>
        </div>
      </div>
      <div className="mt-4 grid grid-cols-2 gap-3">
        <Stat label="Érték">
          <Huf value={b.equity_huf} />
        </Stat>
        <Stat label={`Készpénz (${b.currency})`}>
          <Huf value={b.cash_huf} />
        </Stat>
      </div>
      <div className="mt-3 text-[11.5px] text-subtle">Szinkron: {b.last_sync ? dateTime(b.last_sync) : "–"}</div>
    </Card>
  );
}
