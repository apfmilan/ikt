"use client";

import { useState } from "react";
import { Huf, Kind } from "@/components/money";
import { PageHeader, Section, Stat } from "@/components/page-header";
import { ErrorState, LoadingRows } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input, Label } from "@/components/ui/input";
import { api, ApiError } from "@/lib/api";
import { huf, instrumentLabel } from "@/lib/format";
import { useResource } from "@/lib/live";
import type { PlanConfig } from "@/lib/types";
import { friendlyWebAuthnError, stepUp } from "@/lib/webauthn";

export default function PlanPage() {
  const plan = useResource(api.plan, []);
  const [edit, setDraft] = useState<PlanConfig | null>(null);
  const draft = edit ?? plan.data?.config ?? null;
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const splitValid = draft ? Math.abs(Number(draft.reinvest_pct) + Number(draft.withdraw_pct) - 100) < 1e-9 : false;

  async function save() {
    if (!draft) return;
    setBusy(true);
    setMsg(null);
    try {
      const token = await stepUp("plan_change");
      await api.updatePlan(draft, token);
      setMsg({ ok: true, text: "Mentve." });
      setDraft(null);
      plan.reload();
    } catch (e) {
      setMsg({ ok: false, text: e instanceof ApiError ? e.message : friendlyWebAuthnError(e) });
    } finally {
      setBusy(false);
    }
  }

  const p = plan.data;
  return (
    <div className="animate-fade-up">
      <PageHeader title="Pénzügyi terv" subtitle="Befizetések a nyereségtől elkülönítve. Kivét csak kézzel, a bot soha nem kezdeményez." />
      {plan.loading ? (
        <LoadingRows rows={4} />
      ) : plan.error ? (
        <ErrorState error={plan.error} onRetry={plan.reload} />
      ) : p ? (
        <>
          <Card className="grid grid-cols-2 gap-5 p-5 sm:grid-cols-4">
            <Stat label="Tervezett befizetés eddig">
              <Huf value={p.progress.planned_total_huf} />
            </Stat>
            <Stat label="Tényleges befizetés">
              <Huf value={p.progress.deposited_total_huf} />
            </Stat>
            <Stat label="Kereskedési nyereség">
              <Huf value={p.progress.trading_profit_total_huf} signed />
            </Stat>
            <Stat label="Kivehető (kézzel)" hint="a beállított arány szerint">
              <Huf value={p.progress.withdrawable_huf} />
            </Stat>
          </Card>

          <Section title="Ütemterv">
            <Card className="overflow-x-auto">
              <table className="w-full min-w-[520px] text-[13px]">
                <thead className="text-left text-[12px] text-muted">
                  <tr className="border-b border-line">
                    <th className="px-4 py-3 font-medium">Hónap</th>
                    <th className="px-3 py-3 text-right font-medium">Kereskedés (terv)</th>
                    <th className="px-3 py-3 text-right font-medium">Hosszú táv (terv)</th>
                    <th className="px-4 py-3 text-right font-medium">
                      <span className="inline-flex items-center gap-1.5">
                        Befizetve <Kind kind="realized" />
                      </span>
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-line">
                  {p.schedule.map((m) => (
                    <tr key={m.month}>
                      <td className="px-4 py-2.5 font-medium">{m.month}</td>
                      <td className="num px-3 py-2.5 text-right">{huf(m.planned_trading_huf)}</td>
                      <td className="num px-3 py-2.5 text-right">{huf(m.planned_long_term_huf)}</td>
                      <td className="num px-4 py-2.5 text-right">{huf(m.actual_deposits_huf)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </Card>
          </Section>

          {draft && (
            <Section title="Beállítások">
              <Card className="space-y-4 p-5">
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field label="Kezdő tőke (Ft)" value={draft.initial_huf} onChange={(v) => setDraft({ ...draft, initial_huf: v })} />
                  <Field label="Havi befizetés 2027 áprilisig (Ft)" value={draft.monthly_until_apr_2027_huf} onChange={(v) => setDraft({ ...draft, monthly_until_apr_2027_huf: v })} />
                  <Field label="Havi befizetés 2027 májustól (Ft)" value={draft.monthly_from_may_2027_huf} onChange={(v) => setDraft({ ...draft, monthly_from_may_2027_huf: v })} />
                  <Field label="Hosszú távú arány (%)" value={draft.long_term_share_pct} onChange={(v) => setDraft({ ...draft, long_term_share_pct: v })} />
                  <Field label="Nyereség újrabefektetése (%)" value={draft.reinvest_pct} onChange={(v) => setDraft({ ...draft, reinvest_pct: v })} />
                  <Field label="Nyereség kivétre (%)" value={draft.withdraw_pct} onChange={(v) => setDraft({ ...draft, withdraw_pct: v })} />
                </div>
                {!splitValid && <p className="text-[12.5px] text-warn">Az újrabefektetés és a kivét összege 100% kell legyen.</p>}
                {draft.etf_targets.length > 0 && (
                  <div>
                    <div className="mb-2 text-[13px] text-muted">ETF célsúlyok (hosszú táv, {draft.long_term_start_month}-tól)</div>
                    <div className="flex flex-wrap gap-2">
                      {draft.etf_targets.map((t) => (
                        <span key={t.instrument} className="num rounded-xl bg-surface-2 px-3 py-1.5 text-[13px]">
                          {instrumentLabel(t.instrument).symbol} · {t.weight_pct}%
                        </span>
                      ))}
                    </div>
                  </div>
                )}
                <Button variant="accent" disabled={busy || !splitValid} onClick={save}>
                  {busy ? "Mentés…" : "Mentés (Face ID)"}
                </Button>
                {msg && <p className={msg.ok ? "text-[13px] text-gain" : "text-[13px] text-loss"}>{msg.text}</p>}
              </Card>
            </Section>
          )}
        </>
      ) : null}
    </div>
  );
}

function Field({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const id = label.replace(/\W+/g, "-");
  return (
    <div>
      <Label htmlFor={id}>{label}</Label>
      <Input id={id} inputMode="decimal" value={value} onChange={(e) => onChange(e.target.value.replace(/[^\d.]/g, ""))} />
    </div>
  );
}
