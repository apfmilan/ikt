import { huf, pct, tone } from "@/lib/format";
import { cn } from "@/lib/utils";
import { Badge } from "./ui/badge";

/** HUF amount. `signed` adds +/− and colours by sign. */
export function Huf({ value, signed = false, className }: { value: string | number | null | undefined; signed?: boolean; className?: string }) {
  const t = signed ? tone(value) : "flat";
  return (
    <span className={cn("num", t === "gain" && "text-gain", t === "loss" && "text-loss", className)}>{huf(value, signed)}</span>
  );
}

export function Pct({ value, className }: { value: string | number | null | undefined; className?: string }) {
  const t = tone(value);
  return <span className={cn("num", t === "gain" && "text-gain", t === "loss" && "text-loss", className)}>{pct(value, true)}</span>;
}

/** Explicit provenance label so realized results and forecasts are never confused. */
export function Kind({ kind }: { kind: "realized" | "forecast" }) {
  return kind === "realized" ? <Badge tone="realized">Tényleges</Badge> : <Badge tone="forecast">Előrejelzés</Badge>;
}
