"use client";

import { ArrowLeft } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { PriceChart } from "@/components/charts";
import { DecisionItem } from "@/components/decision";
import { Huf, Kind, Pct } from "@/components/money";
import { PageHeader, Section } from "@/components/page-header";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Card } from "@/components/ui/card";
import { Segmented } from "@/components/ui/segmented";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { BROKER_LABEL, instrumentLabel, price, qty } from "@/lib/format";
import { useResource } from "@/lib/live";

type Interval = "15m" | "1h" | "4h" | "1d";

export default function InstrumentPage() {
  return (
    <Suspense fallback={<LoadingRows rows={3} />}>
      <Instrument />
    </Suspense>
  );
}

function Instrument() {
  const id = useSearchParams().get("id") ?? "";
  const [interval, setInterval] = useState<Interval>("1h");
  const chart = useResource(id ? () => api.chart(id, interval, 300) : null, ["position", "order"], [id, interval]);
  const positions = useResource(api.positions, ["position"]);
  const decisions = useResource(id ? () => api.decisions(20, id) : null, ["decision"], [id]);
  const pos = positions.data?.find((p) => p.instrument === id);
  const { broker, symbol } = instrumentLabel(id);

  if (!id) return <EmptyState title="Nincs kiválasztott eszköz" />;

  return (
    <div className="animate-fade-up">
      <Link href="/positions/" className="mb-3 inline-flex items-center gap-1 text-[13px] text-muted hover:text-fg">
        <ArrowLeft size={14} /> Vissza
      </Link>
      <PageHeader title={symbol} subtitle={BROKER_LABEL[broker] ?? broker} />

      <Card className="overflow-hidden">
        <div className="flex items-center justify-between p-3">
          <Segmented
            value={interval}
            onChange={setInterval}
            options={[
              { value: "15m", label: "15p" },
              { value: "1h", label: "1ó" },
              { value: "4h", label: "4ó" },
              { value: "1d", label: "1n" },
            ]}
          />
        </div>
        {chart.loading ? (
          <Skeleton className="mx-3 mb-3 h-[360px]" />
        ) : chart.error ? (
          <div className="p-3">
            <ErrorState error={chart.error} onRetry={chart.reload} />
          </div>
        ) : chart.data ? (
          <PriceChart data={chart.data} />
        ) : null}
      </Card>

      {pos && (
        <Section title="Pozíció">
          <Card className="grid grid-cols-2 gap-4 p-4 sm:grid-cols-4">
            <Info label="Mennyiség" value={qty(pos.quantity)} />
            <Info label="Átlagár" value={price(pos.avg_price, pos.currency)} />
            <Info label="Stop" value={price(pos.stop_price, pos.currency)} />
            <Info label="Trailing stop" value={price(pos.trailing_stop, pos.currency)} />
            <div className="col-span-2 flex items-center gap-2 sm:col-span-4">
              <Kind kind="realized" />
              <span className="text-[13px] text-muted">Nem realizált:</span>
              <Huf value={pos.unrealized_huf} signed className="font-semibold" />
              <Pct value={pos.unrealized_pct} />
            </div>
          </Card>
        </Section>
      )}

      <Section title="Döntések erről az eszközről">
        {decisions.loading ? (
          <LoadingRows rows={2} />
        ) : decisions.error ? (
          <ErrorState error={decisions.error} onRetry={decisions.reload} />
        ) : (decisions.data ?? []).length === 0 ? (
          <EmptyState title="Nincs döntés" />
        ) : (
          <Card className="divide-y divide-line">
            {(decisions.data ?? []).map((d) => (
              <DecisionItem key={d.id} d={d} />
            ))}
          </Card>
        )}
      </Section>
    </div>
  );
}

function Info({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[12px] text-muted">{label}</div>
      <div className="num mt-0.5 font-medium">{value}</div>
    </div>
  );
}
