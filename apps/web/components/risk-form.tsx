"use client";

import { Input } from "./ui/input";
import type { RiskConfig } from "@/lib/types";

/** Main user-adjustable ceilings. Fractions are shown as percentages. */
export const RISK_FIELDS: { key: string; label: string; unit: "pct" | "count" | "bps"; hint: string }[] = [
  { key: "risk_per_trade", label: "Kockázat ügyletenként", unit: "pct", hint: "A tőke ennyi %-át veszítheti egy ügylet a stopig." },
  { key: "daily_loss", label: "Napi veszteséglimit", unit: "pct", hint: "Elérésekor aznap nincs új belépés." },
  { key: "weekly_loss", label: "Heti veszteséglimit", unit: "pct", hint: "Elérésekor újraengedélyezés kell." },
  { key: "drawdown_halt", label: "Max. visszaesés (leállás)", unit: "pct", hint: "A NAV-csúcstól mért esés, ami leállítja a belépéseket." },
  { key: "position_cap", label: "Max. pozícióméret", unit: "pct", hint: "Egy eszköz a tőke ennyi %-a lehet." },
  { key: "crypto_cap", label: "Max. kripto kitettség", unit: "pct", hint: "" },
  { key: "equity_cap", label: "Max. részvény kitettség", unit: "pct", hint: "" },
  { key: "total_cap", label: "Max. teljes kitettség", unit: "pct", hint: "" },
  { key: "max_positions", label: "Nyitott pozíciók", unit: "count", hint: "" },
  { key: "max_entries_per_day", label: "Belépés naponta", unit: "count", hint: "" },
];

const num = (v: string | number | undefined) => (v === undefined ? NaN : typeof v === "number" ? v : Number(v));

export function toDisplay(unit: string, v: string | number | undefined): string {
  const n = num(v);
  if (!Number.isFinite(n)) return "";
  return unit === "pct" ? String(Math.round(n * 10000) / 100) : String(n);
}

export function fromDisplay(unit: string, s: string): string | number {
  const n = Number(s.replace(",", "."));
  if (unit === "pct") return String(Math.round(n * 100) / 10000);
  return unit === "count" ? Math.round(n) : n;
}

/** Validation against hard limits (the server enforces them too). Floors such as
 *  edge_buffer_bps are not editable here. */
export function riskErrors(config: RiskConfig, hardMax: Record<string, string | number>): Record<string, string> {
  const errs: Record<string, string> = {};
  for (const f of RISK_FIELDS) {
    const v = num(config[f.key]);
    const max = num(hardMax[f.key]);
    if (!Number.isFinite(v) || v < 0) errs[f.key] = "Érvénytelen érték";
    else if (Number.isFinite(max) && v > max) errs[f.key] = `Legfeljebb ${toDisplay(f.unit, max)}${f.unit === "pct" ? "%" : ""}`;
    else if (f.unit === "count" && v < 1) errs[f.key] = "Legalább 1";
  }
  return errs;
}

export function RiskForm({
  config,
  hardMax,
  onChange,
}: {
  config: RiskConfig;
  hardMax: Record<string, string | number>;
  onChange: (c: RiskConfig) => void;
}) {
  const errs = riskErrors(config, hardMax);
  return (
    <div className="divide-y divide-line">
      {RISK_FIELDS.filter((f) => config[f.key] !== undefined).map((f) => (
        <div key={f.key} className="flex items-center gap-4 py-3">
          <div className="min-w-0 flex-1">
            <label htmlFor={`risk-${f.key}`} className="text-[14px] font-medium">
              {f.label}
            </label>
            <div className="text-[12px] text-subtle">
              {f.hint}
              {hardMax[f.key] !== undefined && (
                <span className="ml-1 text-muted">
                  Kemény plafon: {toDisplay(f.unit, hardMax[f.key])}
                  {f.unit === "pct" ? "%" : ""}
                </span>
              )}
            </div>
            {errs[f.key] && <div className="mt-1 text-[12px] text-loss">{errs[f.key]}</div>}
          </div>
          <div className="relative w-28 shrink-0">
            <Input
              id={`risk-${f.key}`}
              inputMode="decimal"
              className="num h-10 pr-8 text-right"
              value={toDisplay(f.unit, config[f.key])}
              onChange={(e) => onChange({ ...config, [f.key]: fromDisplay(f.unit, e.target.value || "0") })}
            />
            {f.unit === "pct" && <span className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-[13px] text-subtle">%</span>}
          </div>
        </div>
      ))}
    </div>
  );
}
