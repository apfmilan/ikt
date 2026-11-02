"use client";

import { ChevronDown, CircleCheck, CircleSlash, Hourglass } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { bps, dateTime, frac, instrumentLabel, qty } from "@/lib/format";
import type { DecisionRow } from "@/lib/types";
import { cn } from "@/lib/utils";
import { Kind } from "./money";
import { Badge } from "./ui/badge";

const ACTION: Record<DecisionRow["action"], { label: string; tone: "accent" | "loss" | "neutral" | "warn" }> = {
  enter: { label: "Belépés", tone: "accent" },
  exit: { label: "Kilépés", tone: "loss" },
  hold: { label: "Tartás", tone: "neutral" },
  abstain: { label: "Tartózkodás", tone: "neutral" },
};

const AGENT_LABEL: Record<string, string> = {
  scanner: "Piacfigyelő",
  technical: "Technikai",
  regime: "Rezsim",
  volatility: "Volatilitás",
  news: "Hírek",
  listings: "Új listázás",
  ranker: "Rangsoroló",
  portfolio: "Portfólió",
  exit: "Kilépés",
  risk: "Kockázat",
  execution: "Végrehajtás",
  reconciler: "Egyeztetés",
  long_term_invest: "Hosszú táv",
  revolut_watch: "Revolut figyelő",
};

export function agentLabel(a: string) {
  return AGENT_LABEL[a] ?? a;
}

/** One explainable decision: action, forecast (clearly labelled), consensus and risk verdict. */
export function DecisionItem({ d, compact = false }: { d: DecisionRow; compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const a = ACTION[d.action];
  const sym = instrumentLabel(d.instrument).symbol;
  const verdict = d.risk?.verdict;

  return (
    <div className="p-4">
      <button className="flex w-full items-start gap-3 text-left" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <VerdictIcon verdict={verdict} action={d.action} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[15px] font-semibold">{sym}</span>
            <Badge tone={a.tone}>{a.label}</Badge>
            {verdict === "veto" && <Badge tone="loss">Kockázati vétó</Badge>}
            {verdict === "allow_resized" && <Badge tone="warn">Méret csökkentve</Badge>}
          </div>
          <p className="mt-1 line-clamp-2 text-[13px] leading-snug text-muted">{d.rationale[0] ?? "—"}</p>
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-subtle">
            <span>{dateTime(d.created_at)}</span>
            <span>{d.strategy}</span>
            <span className="num">Bizalom {frac(d.confidence, 0)}</span>
          </div>
        </div>
        {!compact && <ChevronDown size={18} className={cn("mt-1 shrink-0 text-subtle transition-transform", open && "rotate-180")} />}
      </button>

      {open && !compact && (
        <div className="mt-4 space-y-4 pl-9 animate-fade-up">
          <div className="rounded-2xl border border-forecast/20 bg-forecast/[0.04] p-3.5">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-[13px] font-medium">Várható kimenet</span>
              <Kind kind="forecast" />
            </div>
            <div className="grid grid-cols-3 gap-3 text-[13px]">
              <Metric label="Nyerési esély" value={frac(d.forecast.p_win, 0)} />
              <Metric label="Várható nettó" value={bps(d.forecast.expected_net_bps)} />
              <Metric label="Költség" value={bps(d.forecast.cost_bps, false)} />
              <Metric label="Bizonytalanság" value={bps(d.forecast.uncertainty_bps, false)} />
              <Metric label="5% eset" value={bps(d.forecast.p05_bps)} />
              <Metric label="95% eset" value={bps(d.forecast.p95_bps)} />
            </div>
            <p className="mt-2 text-[11.5px] text-subtle">Becslés, nem garantált eredmény.</p>
          </div>

          {d.rationale.length > 1 && (
            <ul className="space-y-1.5 text-[13px] text-muted">
              {d.rationale.map((r, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-subtle">•</span>
                  {r}
                </li>
              ))}
            </ul>
          )}

          {d.signals.length > 0 && (
            <div>
              <div className="mb-2 text-[12px] font-medium uppercase tracking-[0.08em] text-muted">Ügynökök konszenzusa</div>
              <div className="space-y-1.5">
                {d.signals.map((s, i) => (
                  <div key={i} className="flex items-center gap-3 rounded-xl bg-surface-2 px-3 py-2 text-[13px]">
                    <span className="w-24 shrink-0 font-medium">{agentLabel(s.agent)}</span>
                    <StrengthBar v={s.strength} />
                    <span className="num w-14 shrink-0 text-right text-muted">{frac(s.confidence, 0)}</span>
                    <span className="hidden min-w-0 flex-1 truncate text-subtle sm:block">{s.rationale}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {d.risk && (
            <div className="rounded-2xl border border-line p-3.5">
              <div className="mb-1 flex items-center justify-between text-[13px]">
                <span className="font-medium">Kockázatkezelő döntése</span>
                <span className="num text-muted">{d.risk.quantity ? `${qty(d.risk.quantity)} db` : ""}</span>
              </div>
              {d.risk.vetoes.length === 0 ? (
                <p className="text-[13px] text-gain">Minden ellenőrzés teljesült.</p>
              ) : (
                <ul className="space-y-1 text-[12.5px]">
                  {d.risk.vetoes.map((v) => (
                    <li key={v.code + v.detail}>
                      <span className="font-mono text-loss">{v.code}</span> <span className="text-muted">{v.detail}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
          <Link href={`/instrument/?id=${encodeURIComponent(d.instrument)}`} className="inline-block text-[13px] font-medium text-accent">
            Grafikon megnyitása
          </Link>
        </div>
      )}
    </div>
  );
}

function VerdictIcon({ verdict, action }: { verdict?: string; action: DecisionRow["action"] }) {
  if (verdict === "veto") return <CircleSlash size={22} className="mt-0.5 shrink-0 text-loss" aria-label="Vétó" />;
  if (action === "abstain" || action === "hold") return <Hourglass size={22} className="mt-0.5 shrink-0 text-subtle" aria-label="Nincs művelet" />;
  return <CircleCheck size={22} className="mt-0.5 shrink-0 text-accent" aria-label="Jóváhagyva" />;
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[11.5px] text-subtle">{label}</div>
      <div className="num mt-0.5 font-medium">{value}</div>
    </div>
  );
}

function StrengthBar({ v }: { v: number }) {
  const w = Math.min(1, Math.abs(v)) * 50;
  return (
    <div className="relative h-1.5 flex-1 rounded-full bg-surface-3" aria-label={`Erősség ${v.toFixed(2)}`}>
      <div className="absolute top-0 left-1/2 h-full w-px bg-line-strong" />
      <div
        className={cn("absolute top-0 h-full rounded-full", v >= 0 ? "bg-gain" : "bg-loss")}
        style={v >= 0 ? { left: "50%", width: `${w}%` } : { right: "50%", width: `${w}%` }}
      />
    </div>
  );
}
