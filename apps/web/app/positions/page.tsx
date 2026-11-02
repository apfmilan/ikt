"use client";

import { ChevronRight } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { Huf, Kind, Pct } from "@/components/money";
import { PageHeader, Section } from "@/components/page-header";
import { SellSheet } from "@/components/sell-sheet";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { api } from "@/lib/api";
import { BROKER_LABEL, dateTime, price, qty } from "@/lib/format";
import { useResource } from "@/lib/live";
import type { PositionRow } from "@/lib/types";

export default function PositionsPage() {
  const positions = useResource(api.positions, ["position", "order"]);
  const orders = useResource(() => api.orders("open"), ["order"]);
  const [selling, setSelling] = useState<PositionRow | null>(null);

  const trading = (positions.data ?? []).filter((p) => !p.long_term);
  const longTerm = (positions.data ?? []).filter((p) => p.long_term);

  return (
    <div className="animate-fade-up">
      <PageHeader title="Pozíciók" subtitle={<span className="flex items-center gap-2">Nem realizált eredmény <Kind kind="realized" /></span>} />
      {positions.loading ? (
        <LoadingRows rows={3} />
      ) : positions.error ? (
        <ErrorState error={positions.error} onRetry={positions.reload} />
      ) : (positions.data ?? []).length === 0 ? (
        <EmptyState title="Nincs nyitott pozíció" hint="A bot csak akkor lép be, ha a várható előny meghaladja a költségeket." />
      ) : (
        <>
          <PositionList rows={trading} onSell={setSelling} />
          {longTerm.length > 0 && (
            <Section title="Hosszú távú befektetések">
              <PositionList rows={longTerm} onSell={setSelling} />
            </Section>
          )}
        </>
      )}

      <Section title="Nyitott megbízások">
        {orders.loading ? (
          <LoadingRows rows={2} />
        ) : orders.error ? (
          <ErrorState error={orders.error} onRetry={orders.reload} />
        ) : (orders.data ?? []).length === 0 ? (
          <EmptyState title="Nincs nyitott megbízás" />
        ) : (
          <Card className="divide-y divide-line">
            {(orders.data ?? []).map((o) => (
              <div key={o.id} className="flex items-center gap-3 p-4 text-[13px]">
                <Badge tone={o.side === "buy" ? "accent" : "loss"}>{o.side === "buy" ? "Vétel" : "Eladás"}</Badge>
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium">{o.instrument.split(":")[1] ?? o.instrument}</div>
                  <div className="text-subtle">
                    {o.kind} · {o.reason} · {dateTime(o.created_at)}
                  </div>
                </div>
                <div className="text-right">
                  <div className="num">{qty(o.quantity)}</div>
                  <div className="text-subtle">{o.state}</div>
                </div>
              </div>
            ))}
          </Card>
        )}
      </Section>

      {selling && <SellSheet position={selling} onClose={() => setSelling(null)} onDone={positions.reload} />}
    </div>
  );
}

function PositionList({ rows, onSell }: { rows: PositionRow[]; onSell: (p: PositionRow) => void }) {
  return (
    <div className="space-y-3">
      {rows.map((p) => (
        <Card key={p.instrument} className="p-4">
          <div className="flex items-start gap-3">
            <Link href={`/instrument/?id=${encodeURIComponent(p.instrument)}`} className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-[16px] font-semibold">{p.symbol}</span>
                <Badge>{BROKER_LABEL[p.broker] ?? p.broker}</Badge>
                <ChevronRight size={15} className="text-subtle" />
              </div>
              <div className="mt-0.5 truncate text-[12.5px] text-muted">{p.name}</div>
            </Link>
            <div className="text-right">
              <Huf value={p.value_huf} className="text-[16px] font-semibold" />
              <div className="mt-0.5 text-[13px]">
                <Huf value={p.unrealized_huf} signed /> <Pct value={p.unrealized_pct} className="ml-1" />
              </div>
            </div>
          </div>
          <div className="mt-3 grid grid-cols-4 gap-2 text-[12px]">
            <Cell label="Mennyiség" value={qty(p.quantity)} />
            <Cell label="Átlagár" value={price(p.avg_price)} />
            <Cell label="Stop" value={price(p.stop_price)} tone="loss" />
            <Cell label="Cél" value={price(p.take_profit)} tone="gain" />
          </div>
          <div className="mt-3 flex items-center justify-between">
            <span className="text-[12px] text-subtle">
              {p.strategy} · {dateTime(p.opened_at)}
            </span>
            <Button variant="destructive-soft" size="sm" onClick={() => onSell(p)}>
              Eladás
            </Button>
          </div>
        </Card>
      ))}
    </div>
  );
}

function Cell({ label, value, tone }: { label: string; value: string; tone?: "gain" | "loss" }) {
  return (
    <div className="rounded-xl bg-surface-2 px-2.5 py-2">
      <div className="text-subtle">{label}</div>
      <div className={`num mt-0.5 font-medium ${tone === "gain" ? "text-gain" : tone === "loss" ? "text-loss" : ""}`}>{value}</div>
    </div>
  );
}
