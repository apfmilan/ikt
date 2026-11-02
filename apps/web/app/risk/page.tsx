"use client";

import { Clock } from "lucide-react";
import { useState } from "react";
import { Pct } from "@/components/money";
import { PageHeader, Section, Stat } from "@/components/page-header";
import { RiskForm, riskErrors } from "@/components/risk-form";
import { ErrorState, LoadingRows } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { api, ApiError } from "@/lib/api";
import { dateTime } from "@/lib/format";
import { useResource } from "@/lib/live";
import type { RiskConfig } from "@/lib/types";
import { friendlyWebAuthnError, stepUp } from "@/lib/webauthn";

export default function RiskPage() {
  const risk = useResource(api.risk, ["mode", "position"]);
  const [edit, setDraft] = useState<RiskConfig | null>(null);
  const draft = edit ?? risk.data?.config ?? null;
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const r = risk.data;
  const errs = draft && r ? riskErrors(draft, r.hard_max) : {};
  const dirty = draft && r ? JSON.stringify(draft) !== JSON.stringify(r.config) : false;

  async function save() {
    if (!draft) return;
    setBusy(true);
    setMsg(null);
    try {
      const token = await stepUp("risk_change");
      const res = await api.updateRisk(draft, token);
      setMsg({
        ok: true,
        text: res.effective_at
          ? `Lazítás: ${dateTime(res.effective_at)}-kor lép életbe, és az éles engedélyt újra meg kell adni.`
          : "Szigorítás azonnal életbe lépett.",
      });
      setDraft(null);
      risk.reload();
    } catch (e) {
      setMsg({ ok: false, text: e instanceof ApiError ? e.message : friendlyWebAuthnError(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="animate-fade-up">
      <PageHeader title="Kockázatkezelés" subtitle="A determinisztikus kockázatkezelő minden megbízást ellenőriz, és bármelyiket megvétózhatja." />
      {risk.loading ? (
        <LoadingRows rows={4} />
      ) : risk.error ? (
        <ErrorState error={risk.error} onRetry={risk.reload} />
      ) : r && draft ? (
        <>
          <Card className="grid grid-cols-2 gap-5 p-5 sm:grid-cols-3">
            <Stat label="Teljes kitettség">
              <Pct value={r.utilization.exposure_pct} className="text-fg" />
            </Stat>
            <Stat label="Kripto">
              <Pct value={r.utilization.crypto_pct} className="text-fg" />
            </Stat>
            <Stat label="Részvény/ETF">
              <Pct value={r.utilization.equity_pct} className="text-fg" />
            </Stat>
            <Stat label="Mai eredmény">
              <Pct value={r.utilization.day_pnl_pct} />
            </Stat>
            <Stat label="Visszaesés">
              <Pct value={`-${r.utilization.drawdown_pct}`} />
            </Stat>
            <Stat label="Mai belépések">{r.utilization.entries_today}</Stat>
          </Card>

          {r.pending_change && (
            <div className="mt-4 flex items-start gap-3 rounded-2xl border border-warn/30 bg-warn/8 p-4 text-[13px]">
              <Clock size={16} className="mt-0.5 text-warn" />
              <span>Függő lazítás: {dateTime(r.pending_change.effective_at)}-kor lép életbe (24 órás türelmi idő).</span>
            </div>
          )}

          <Section title="Plafonok">
            <Card className="px-5 py-2">
              <RiskForm config={draft} hardMax={r.hard_max} onChange={setDraft} />
            </Card>
            <p className="mt-3 text-[12.5px] leading-snug text-muted">
              Szigorítás azonnal érvényes. Lazítás 24 óra múlva lép életbe és visszavonja az éles engedélyt. A kemény plafonok a kódban rögzítettek; nem
              módosíthatók. Tőkeáttét, short és kivételi jogú kulcs soha nem engedélyezett.
            </p>
            <div className="mt-4 flex gap-2">
              <Button variant="accent" disabled={!dirty || busy || Object.keys(errs).length > 0} onClick={save}>
                {busy ? "Mentés…" : "Mentés (Face ID)"}
              </Button>
              {dirty && (
                <Button variant="ghost" onClick={() => setDraft(null)}>
                  Elvetés
                </Button>
              )}
            </div>
            {msg && <p className={`mt-3 text-[13px] ${msg.ok ? "text-gain" : "text-loss"}`}>{msg.text}</p>}
          </Section>
        </>
      ) : null}
    </div>
  );
}
