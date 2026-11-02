"use client";

import { DailyPnlChart } from "@/components/charts";
import { Huf, Kind, Pct } from "@/components/money";
import { PageHeader, Section, Stat } from "@/components/page-header";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { api } from "@/lib/api";
import { frac } from "@/lib/format";
import { useResource } from "@/lib/live";

export default function PerformancePage() {
  const perf = useResource(api.performance, ["order", "position"]);
  const r = perf.data?.realized;

  return (
    <div className="animate-fade-up">
      <PageHeader title="Teljesítmény" subtitle={<span className="flex items-center gap-2">Kizárólag lezárt ügyletekből <Kind kind="realized" /></span>} />
      {perf.loading ? (
        <LoadingRows rows={4} />
      ) : perf.error ? (
        <ErrorState error={perf.error} onRetry={perf.reload} />
      ) : !perf.data || !r ? null : (
        <>
          <Card className="grid grid-cols-2 gap-5 p-5 sm:grid-cols-4">
            <Stat label="Hozam (NAV)">
              <Pct value={r.total_return_pct} />
            </Stat>
            <Stat label="Nyerési arány">{frac(r.win_rate, 1)}</Stat>
            <Stat label="Profit faktor">{Number.isFinite(r.profit_factor) ? r.profit_factor.toFixed(2) : "–"}</Stat>
            <Stat label="Max. visszaesés">
              <Pct value={`-${r.max_drawdown_pct}`} />
            </Stat>
            <Stat label="Ügyletek">{r.trades}</Stat>
            <Stat label="Díjak összesen">
              <Huf value={r.fees_huf} />
            </Stat>
            <Stat label="Átlag / ügylet">
              <Huf value={r.expectancy_huf} signed />
            </Stat>
          </Card>

          <Section title="Napi nettó eredmény">
            <Card className="p-2">
              <DailyPnlChart days={perf.data.daily} />
            </Card>
          </Section>

          <Section title="Stratégiák összehasonlítása">
            {perf.data.by_strategy.length === 0 ? (
              <EmptyState title="Nincs adat" />
            ) : (
              <Card className="overflow-x-auto">
                <table className="w-full min-w-[640px] text-[13px]">
                  <thead className="text-left text-[12px] text-muted">
                    <tr className="border-b border-line">
                      <th className="px-4 py-3 font-medium">Stratégia</th>
                      <th className="px-3 py-3 text-right font-medium">Ügyletek</th>
                      <th className="px-3 py-3 text-right font-medium">Nyerési arány</th>
                      <th className="px-3 py-3 text-right font-medium">Nettó</th>
                      <th className="px-3 py-3 text-right font-medium">Díjak</th>
                      <th className="px-3 py-3 text-right font-medium">Max DD</th>
                      <th className="px-4 py-3 text-right font-medium">Backteszt OOS (PF / Sharpe)</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-line">
                    {perf.data.by_strategy.map((s) => (
                      <tr key={s.strategy}>
                        <td className="px-4 py-3">
                          <div className="font-medium">{s.strategy}</div>
                          <Badge className="mt-1">{s.status}</Badge>
                        </td>
                        <td className="num px-3 py-3 text-right">{s.trades}</td>
                        <td className="num px-3 py-3 text-right">{frac(s.win_rate, 0)}</td>
                        <td className="px-3 py-3 text-right">
                          <Huf value={s.net_huf} signed />
                        </td>
                        <td className="px-3 py-3 text-right">
                          <Huf value={s.fees_huf} />
                        </td>
                        <td className="px-3 py-3 text-right">
                          <Pct value={`-${s.max_drawdown_pct}`} />
                        </td>
                        <td className="num px-4 py-3 text-right text-muted">
                          {s.backtest_oos ? `${s.backtest_oos.profit_factor.toFixed(2)} / ${s.backtest_oos.sharpe.toFixed(2)}` : "–"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </Card>
            )}
          </Section>
        </>
      )}
    </div>
  );
}
