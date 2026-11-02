"use client";

import { OctagonX, ShieldAlert } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";
import { MODE_LABEL } from "@/lib/format";
import type { KillMode, Mode } from "@/lib/types";
import { friendlyWebAuthnError, stepUp } from "@/lib/webauthn";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { Sheet } from "./ui/sheet";

const OPTIONS: { mode: KillMode; title: string; text: string }[] = [
  { mode: "reduce_only", title: "Csak zárás", text: "Nincs új belépés. A nyitott pozíciók védelme és kilépései működnek." },
  { mode: "flatten", title: "Minden pozíció zárása", text: "Törli a nyitott belépő megbízásokat, majd eladja a kereskedési pozíciókat." },
  { mode: "freeze", title: "Teljes leállítás", text: "A bot semmit nem küld. A brókernél lévő stop megbízások érvényben maradnak." },
];

/** Always-reachable emergency control in the top bar. Stricter modes need only the session;
 *  returning to automatic trading needs a passkey step-up. */
export function EmergencyStop({ mode, onChanged }: { mode?: Mode; onChanged: () => void }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<KillMode | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const stopped = mode !== undefined && mode !== "normal";

  async function apply(m: KillMode) {
    setBusy(m);
    setErr(null);
    try {
      const token = m === "normal" ? await stepUp("kill_release") : m === "flatten" ? await stepUp("flatten") : undefined;
      await api.kill(m, token);
      onChanged();
      setOpen(false);
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : friendlyWebAuthnError(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <>
      <Button
        variant={stopped ? "destructive" : "destructive-soft"}
        size="sm"
        onClick={() => setOpen(true)}
        aria-label="Vészleállítás"
        className="rounded-full px-3.5"
      >
        <OctagonX size={16} />
        <span className="text-[13px]">{stopped ? MODE_LABEL[mode] : "Stop"}</span>
      </Button>
      <Sheet open={open} onClose={() => setOpen(false)} title="Vészleállítás">
        <div className="space-y-2">
          {OPTIONS.map((o) => (
            <button
              key={o.mode}
              disabled={busy !== null}
              onClick={() => apply(o.mode)}
              className={cn(
                "w-full rounded-2xl border p-4 text-left transition-colors disabled:opacity-50",
                mode === o.mode ? "border-loss/60 bg-loss/10" : "border-line bg-surface hover:bg-surface-3",
              )}
            >
              <div className="flex items-center justify-between">
                <span className="text-[15px] font-semibold">{o.title}</span>
                {busy === o.mode && <span className="text-[12px] text-muted">Küldés…</span>}
              </div>
              <p className="mt-1 text-[13px] leading-snug text-muted">{o.text}</p>
            </button>
          ))}
        </div>
        {stopped && (
          <Button className="mt-4 w-full" variant="accent" disabled={busy !== null} onClick={() => apply("normal")}>
            Automata kereskedés visszaállítása (Face ID)
          </Button>
        )}
        {err && (
          <p role="alert" className="mt-3 text-[13px] text-loss">
            {err}
          </p>
        )}
      </Sheet>
    </>
  );
}

export function ModeBanner({ mode, live }: { mode?: Mode; live?: Partial<Record<string, boolean>> }) {
  if (!mode) return null;
  const anyLive = live ? Object.values(live).some(Boolean) : false;
  if (mode === "normal" && anyLive) return null;
  const text =
    mode === "normal"
      ? "Az éles kereskedés még nincs engedélyezve. A bot nem küld megbízást."
      : mode === "recovery"
        ? "Helyreállítás: egyeztetés a brókerekkel. Új belépés nincs."
        : `${MODE_LABEL[mode]} — új belépés nincs.`;
  return (
    <div className={cn("border-b px-4 py-2.5 lg:px-8", mode === "normal" ? "border-warn/20 bg-warn/8" : "border-loss/25 bg-loss/10")}>
      <div className="mx-auto flex max-w-6xl items-center gap-2 text-[13px]">
        <ShieldAlert size={15} className={mode === "normal" ? "text-warn" : "text-loss"} aria-hidden />
        <span className="flex-1">{text}</span>
        {mode === "normal" && (
          <Link href="/onboarding/" className="font-medium text-fg underline-offset-4 hover:underline">
            Élesítés
          </Link>
        )}
      </div>
    </div>
  );
}
