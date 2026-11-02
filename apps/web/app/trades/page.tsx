"use client";

import { useState } from "react";
import { Huf, Kind, Pct } from "@/components/money";
import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Segmented } from "@/components/ui/segmented";
import { api } from "@/lib/api";
import { bps, BROKER_LABEL, dateTime, frac, instrumentLabel, price, qty } from "@/lib/format";
import { useResource } from "@/lib/live";
import type { LedgerKind } from "@/lib/types";

const KIND: Record<LedgerKind, string> = {
  deposit: "Befizetés",
  withdrawal: "Kivét",
  fee: "Díj",
  realized_pnl: "Realizált eredmény",
  dividend: "Osztalék",
  interest: "Kamat",
  fx: "Devizaváltás",
};

export default function TradesPage() {
  const [tab, setTab] = useState<"trades" | "ledger">("trades");
  const trades = useResource(tab === "trades" ? () => api.trades(200) : null, ["order", "position"], [tab]);
  const ledger = useResource(tab === "ledger" ? () => api.transactions(200) : null, ["order"], [tab]);

  return (
    <div className="animate-fade-up">
      <PageHeader title="Kereskedések" subtitle="Lezárt ügyletek és teljes pénzügyi napló." />
      <Segmented
        className="mb-4"
        value={tab}
        onChange={setTab}
        options={[
          { value: "trades", label: "Ügyletek" },
          { value: "ledger", label: "Tranzakciók" },
        ]}
      />
      {tab === "trades" ? (
        trades.loading ? (
          <LoadingRows rows={4} />
        ) : trades.error ? (
          <ErrorState error={trades.error} onRetry={trades.reload} />
        ) : (trades.data ?? []).length === 0 ? (
          <EmptyState title="Még nincs lezárt ügylet" />
        ) : (
          <div className="space-y-3">
            {(trades.data ?? []).map((t) => (
              <Card key={t.id} className="p-4">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <div className="flex items-center gap-2">
                      <span className="text-[15px] font-semibold">{instrumentLabel(t.instrument).symbol}</span>
                      <Badge>{t.strategy}</Badge>
                      <Badge tone={t.exit_reason === "stop_loss" ? "loss" : "neutral"}>{t.exit_reason}</Badge>
                    </div>
                    <div className="mt-1 text-[12px] text-subtle">
                      {dateTime(t.entry_at)} → {dateTime(t.exit_at)}
                    </div>
                  </div>
                  <div className="text-right">
                    <Huf value={t.realized.net_huf} signed className="text-[16px] font-semibold" />
                    <div className="text-[12px]">
                      <Pct value={t.realized.return_pct} />
                    </div>
                  </div>
                </div>
                <div className="mt-3 grid grid-cols-2 gap-3 border-t border-line pt-3 text-[12px] sm:grid-cols-4">
                  <div>
                    <div className="mb-1 flex items-center gap-1.5 text-subtle">
                      Eredmény <Kind kind="realized" />
                    </div>
                    <div className="num">
                      {price(t.entry_price)} → {price(t.exit_price)} · {qty(t.quantity)}
                    </div>
                  </div>
                  <div>
                    <div className="mb-1 text-subtle">Díjak</div>
                    <Huf value={t.realized.fees_huf} />
                  </div>
                  {t.forecast && (
                    <div className="col-span-2">
                      <div className="mb-1 flex items-center gap-1.5 text-subtle">
                        Belépéskori becslés <Kind kind="forecast" />
                      </div>
                      <div className="num text-forecast">
                        {frac(t.forecast.p_win, 0)} esély · {bps(t.forecast.expected_net_bps)}
                      </div>
                    </div>
                  )}
                </div>
              </Card>
            ))}
          </div>
        )
      ) : ledger.loading ? (
        <LoadingRows rows={4} />
      ) : ledger.error ? (
        <ErrorState error={ledger.error} onRetry={ledger.reload} />
      ) : (ledger.data ?? []).length === 0 ? (
        <EmptyState title="Nincs tranzakció" />
      ) : (
        <Card className="divide-y divide-line">
          {(ledger.data ?? []).map((x) => (
            <div key={x.id} className="flex items-center gap-3 p-4">
              <div className="min-w-0 flex-1">
                <div className="text-[14px] font-medium">{KIND[x.kind] ?? x.kind}</div>
                <div className="text-[12px] text-subtle">
                  {BROKER_LABEL[x.broker] ?? x.broker} · {dateTime(x.ts)}
                </div>
              </div>
              <div className="text-right">
                <Huf value={x.amount_huf} signed={x.kind !== "deposit" && x.kind !== "withdrawal"} className="font-medium" />
                {x.currency !== "HUF" && (
                  <div className="num text-[12px] text-subtle">
                    {price(x.amount)} {x.currency}
                  </div>
                )}
              </div>
            </div>
          ))}
        </Card>
      )}
    </div>
  );
}
