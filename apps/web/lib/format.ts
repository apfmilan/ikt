// Display formatting. Values arrive as decimal strings; conversion to number happens only
// here, for presentation.

const TZ = "Europe/Budapest";

const hufFmt = new Intl.NumberFormat("hu-HU", {
  useGrouping: "always",
  style: "currency",
  currency: "HUF",
  maximumFractionDigits: 0,
  minimumFractionDigits: 0,
});
const hufFmtSigned = new Intl.NumberFormat("hu-HU", {
  useGrouping: "always",
  style: "currency",
  currency: "HUF",
  maximumFractionDigits: 0,
  minimumFractionDigits: 0,
  signDisplay: "always",
});

export function toNum(v: string | number | null | undefined): number | null {
  if (v === null || v === undefined || v === "") return null;
  const n = typeof v === "number" ? v : Number(v);
  return Number.isFinite(n) ? n : null;
}

/** `1 000 Ft`, or `+1 000 Ft` with `signed`. Zero is shown as `0 Ft` (no sign). */
export function huf(v: string | number | null | undefined, signed = false): string {
  const n = toNum(v);
  if (n === null) return "–";
  const r = Math.round(n);
  if (signed && r !== 0) return hufFmtSigned.format(r).replace("-", "−");
  return hufFmt.format(r === 0 ? 0 : r).replace("-", "−");
}

export function pct(v: string | number | null | undefined, signed = false, digits = 2): string {
  const n = toNum(v);
  if (n === null) return "–";
  const s = new Intl.NumberFormat("hu-HU", {
  useGrouping: "always",
    maximumFractionDigits: digits,
    minimumFractionDigits: digits,
    signDisplay: signed ? "exceptZero" : "auto",
  }).format(n);
  return `${s.replace("-", "−")}%`;
}

/** Fraction (0.125) to percent string. */
export function frac(v: number | null | undefined, digits = 1): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "–";
  return pct(v * 100, false, digits);
}

export function bps(v: number | null | undefined, signed = true): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "–";
  const s = new Intl.NumberFormat("hu-HU", {
  useGrouping: "always", maximumFractionDigits: 0, signDisplay: signed ? "exceptZero" : "auto" }).format(v);
  return `${s.replace("-", "−")} bp`;
}

/** Prices with adaptive precision (crypto can be tiny). */
export function price(v: string | number | null | undefined, currency?: string): string {
  const n = toNum(v);
  if (n === null) return "–";
  const abs = Math.abs(n);
  const digits = abs >= 1000 ? 0 : abs >= 1 ? 2 : abs >= 0.01 ? 4 : 8;
  const s = new Intl.NumberFormat("hu-HU", {
  useGrouping: "always", maximumFractionDigits: digits, minimumFractionDigits: Math.min(digits, 2) }).format(n);
  return currency ? `${s} ${currency}` : s;
}

export function qty(v: string | number | null | undefined): string {
  const n = toNum(v);
  if (n === null) return "–";
  return new Intl.NumberFormat("hu-HU", {
  useGrouping: "always", maximumFractionDigits: 8 }).format(n);
}

export function dateTime(iso: string | null | undefined): string {
  if (!iso) return "–";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "–";
  return new Intl.DateTimeFormat("hu-HU", {
    timeZone: TZ,
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(d);
}

export function date(iso: string | null | undefined): string {
  if (!iso) return "–";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "–";
  return new Intl.DateTimeFormat("hu-HU", { timeZone: TZ, year: "numeric", month: "short", day: "numeric" }).format(d);
}

const rtf = new Intl.RelativeTimeFormat("hu-HU", { numeric: "auto" });

export function ago(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "–";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "–";
  const s = Math.round((t - now) / 1000);
  const a = Math.abs(s);
  if (a < 45) return "épp most";
  if (a < 3600) return rtf.format(Math.round(s / 60), "minute");
  if (a < 86400) return rtf.format(Math.round(s / 3600), "hour");
  return rtf.format(Math.round(s / 86400), "day");
}

/** Sign class for gains/losses. */
export function tone(v: string | number | null | undefined): "gain" | "loss" | "flat" {
  const n = toNum(v);
  if (n === null || Math.round(n * 100) === 0) return "flat";
  return n > 0 ? "gain" : "loss";
}

export const BROKER_LABEL: Record<string, string> = {
  trading212: "Trading 212",
  bitvavo: "Bitvavo",
  revolut: "Revolut",
  eodhd: "EODHD",
};

export const MODE_LABEL: Record<string, string> = {
  normal: "Automata",
  reduce_only: "Csak zárás",
  flatten: "Pozíciók zárása",
  freeze: "Leállítva",
  recovery: "Helyreállítás",
};

export function instrumentLabel(id: string): { broker: string; symbol: string } {
  const i = id.indexOf(":");
  if (i < 0) return { broker: "", symbol: id };
  return { broker: id.slice(0, i), symbol: id.slice(i + 1) };
}
