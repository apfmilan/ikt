"use client";

import { Kind } from "@/components/money";
import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Huf } from "@/components/money";
import { api } from "@/lib/api";
import { useResource } from "@/lib/live";
import type { StrategyStatus } from "@/lib/types";

const STATUS: Record<StrategyStatus, { label: string; tone: "neutral" | "accent" | "gain" | "warn" | "loss" }> = {
  disabled: { label: "Kikapcsolva", tone: "neutral" },
  validated: { label: "Validálva", tone: "accent" },
  live_half: { label: "Éles (fél kockázat)", tone: "gain" },
  live: { label: "Éles", tone: "gain" },
  demoted: { label: "Visszaminősítve", tone: "loss" },
};

export default function StrategiesPage() {
  const res = useResource(api.strategies, ["agent"]);
  return (
    <div className="animate-fade-up">
      <PageHeader
        title="Stratégiák"
        subtitle="Csak out-of-sample validált stratégia kereskedhet élesben; romló eredménynél automatikusan visszaminősül."
      />
      {res.loading ? (
        <LoadingRows rows={4} />
      ) : res.error ? (
        <ErrorState error={res.error} onRetry={res.reload} />
      ) : (res.data ?? []).length === 0 ? (
        <EmptyState title="Nincs regisztrált stratégia" />
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {(res.data ?? []).map((s) => {
            const st = STATUS[s.status];
            return (
              <Card key={`${s.name}@${s.version}`} className="p-4">
                <div className="flex items-center justify-between gap-2">
                  <div>
                    <div className="text-[15px] font-semibold">{s.name}</div>
                    <div className="text-[12px] text-subtle">
                      v{s.version} · {s.asset_class} · {s.regimes.join(", ") || "minden rezsim"}
                    </div>
                  </div>
                  <Badge tone={st.tone}>{st.label}</Badge>
                </div>
                <div className="mt-4 rounded-2xl bg-surface-2 p-3">
                  <div className="mb-2 flex items-center justify-between text-[12px] text-muted">
                    Történeti backteszt (OOS, költségekkel)
                    {s.backtest?.eligible ? <Badge tone="gain">Megfelelt</Badge> : <Badge tone="warn">Nem felelt meg</Badge>}
                  </div>
                  {s.backtest ? (
                    <div className="grid grid-cols-4 gap-2 text-[12px]">
                      <M l="Ügylet" v={String(s.backtest.oos_trades)} />
                      <M l="Sharpe" v={s.backtest.oos_sharpe.toFixed(2)} />
                      <M l="PF" v={s.backtest.oos_profit_factor.toFixed(2)} />
                      <M l="Max DD" v={`${s.backtest.oos_max_dd_pct.toFixed(1)}%`} />
                    </div>
                  ) : (
                    <p className="text-[12px] text-subtle">Még nincs backteszt.</p>
                  )}
                </div>
                {s.live && (
                  <div className="mt-3 flex items-center justify-between text-[13px]">
                    <span className="flex items-center gap-2 text-muted">
                      Éles: {s.live.trades} ügylet <Kind kind="realized" />
                    </span>
                    <span>
                      átlag <Huf value={s.live.expectancy_huf} signed />
                    </span>
                  </div>
                )}
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}

function M({ l, v }: { l: string; v: string }) {
  return (
    <div>
      <div className="text-subtle">{l}</div>
      <div className="num font-medium">{v}</div>
    </div>
  );
}
