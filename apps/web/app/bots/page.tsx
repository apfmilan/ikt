"use client";

import { Activity, Gauge, Newspaper, Radar, Rocket, Scale, Shield, Sparkles, Zap, type LucideIcon } from "lucide-react";
import { agentLabel } from "@/components/decision";
import { Huf } from "@/components/money";
import { PageHeader, Section } from "@/components/page-header";
import { useOverview } from "@/components/shell";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge, Dot } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { useResource } from "@/lib/live";
import type { AgentName, AgentStatus } from "@/lib/types";
import { cn } from "@/lib/utils";

const ICON: Partial<Record<AgentName, LucideIcon>> = {
  scanner: Radar,
  technical: Activity,
  regime: Gauge,
  volatility: Zap,
  news: Newspaper,
  listings: Rocket,
  ranker: Sparkles,
  portfolio: Scale,
  risk: Shield,
};

const GROUPS: { title: string; agents: AgentName[] }[] = [
  { title: "Piacelemzés", agents: ["scanner", "technical", "regime", "volatility", "news", "listings"] },
  { title: "Döntés", agents: ["ranker", "portfolio", "exit", "long_term_invest", "revolut_watch"] },
  { title: "Végrehajtás és biztonság", agents: ["risk", "execution", "reconciler"] },
];

export default function BotsPage() {
  const overview = useOverview();
  const agents = useResource(api.agents, ["agent"]);
  const byName = new Map((agents.data ?? []).map((a) => [a.agent, a]));

  return (
    <div className="animate-fade-up">
      <PageHeader title="Botok és ügynökök" subtitle="Minden ügynök determinisztikus; a kockázatkezelőnek abszolút vétójoga van." />

      <div className="grid gap-3 sm:grid-cols-3">
        {(overview.data?.bots ?? []).map((b) => (
          <Card key={b.id} className="p-4">
            <div className="flex items-center justify-between">
              <span className="text-[15px] font-semibold">{b.name}</span>
              <Badge tone={b.status === "active" ? "gain" : b.status === "error" ? "loss" : b.status === "idle" ? "neutral" : "warn"}>
                <Dot className={cn(b.status === "active" && "animate-pulse-dot")} />
                {b.status}
              </Badge>
            </div>
            <Huf value={b.today_pnl_huf} signed className="mt-3 block text-[22px] font-semibold tracking-tight" />
            <p className="mt-1 text-[12.5px] text-muted">{b.message || "—"}</p>
          </Card>
        ))}
      </div>

      {agents.loading ? (
        <div className="mt-6">
          <LoadingRows rows={4} />
        </div>
      ) : agents.error ? (
        <div className="mt-6">
          <ErrorState error={agents.error} onRetry={agents.reload} />
        </div>
      ) : (agents.data ?? []).length === 0 ? (
        <div className="mt-6">
          <EmptyState title="Az ügynökök még nem futottak" hint="Az engine indulás után helyreállító módban egyeztet, majd elindítja őket." />
        </div>
      ) : (
        GROUPS.map((g) => (
          <Section key={g.title} title={g.title}>
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {g.agents.map((name) => {
                const a = byName.get(name);
                return a ? <AgentCard key={name} a={a} /> : null;
              })}
            </div>
          </Section>
        ))
      )}
    </div>
  );
}

function AgentCard({ a }: { a: AgentStatus }) {
  const Icon = ICON[a.agent] ?? Activity;
  const ok = a.status === "ok" || a.status === "active" || a.status === "idle";
  return (
    <Card className="p-4">
      <div className="flex items-center gap-2.5">
        <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-surface-3">
          <Icon size={16} />
        </span>
        <span className="flex-1 text-[14px] font-semibold">{agentLabel(a.agent)}</span>
        <Badge tone={ok ? "gain" : a.status === "error" ? "loss" : "warn"}>{a.status}</Badge>
      </div>
      <p className="mt-3 text-[13px] leading-snug text-muted">{a.summary || "—"}</p>
      {Object.keys(a.metrics).length > 0 && (
        <dl className="mt-3 grid grid-cols-2 gap-2 text-[12px]">
          {Object.entries(a.metrics)
            .slice(0, 6)
            .map(([k, v]) => (
              <div key={k} className="rounded-lg bg-surface-2 px-2.5 py-1.5">
                <dt className="truncate text-subtle">{k}</dt>
                <dd className="num truncate font-medium">{v === null ? "–" : String(v)}</dd>
              </div>
            ))}
        </dl>
      )}
      <div className="mt-3 flex justify-between text-[11.5px] text-subtle">
        <span>Utolsó futás: {a.last_run_at ? ago(a.last_run_at) : "–"}</span>
        <span className="num">{a.last_duration_ms !== null ? `${a.last_duration_ms} ms` : ""}</span>
      </div>
    </Card>
  );
}
