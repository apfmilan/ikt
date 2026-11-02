"use client";

import { CircleAlert, CircleCheck, CircleDashed, Lock, ScanFace } from "lucide-react";
import Link from "next/link";
import { useState, type ReactNode } from "react";
import { PageHeader } from "@/components/page-header";
import { RiskForm, riskErrors } from "@/components/risk-form";
import { ErrorState, LoadingRows } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input, Label } from "@/components/ui/input";
import { api, ApiError } from "@/lib/api";
import { useResource } from "@/lib/live";
import type { Onboarding, OnboardingStepId, RiskConfig } from "@/lib/types";
import { friendlyWebAuthnError, stepUp } from "@/lib/webauthn";
import { cn } from "@/lib/utils";

const TITLES: Record<OnboardingStepId, { title: string; text: string }> = {
  passkey: { title: "Passkey", text: "Face ID / Windows Hello belépés." },
  trading212: { title: "Trading 212", text: "Részvények és ETF-ek (Invest vagy Stocks ISA számla)." },
  bitvavo: { title: "Bitvavo", text: "Kriptó (EUR piacok)." },
  market_data: { title: "Piaci adatok (EODHD)", text: "Részvény/ETF árfolyamok — a Trading 212 API nem ad piaci adatot." },
  risk: { title: "Kockázati plafonok", text: "Konzervatív alapértékek; a kemény plafonok nem léphetők túl." },
  plan: { title: "Pénzügyi terv", text: "Befizetési ütemterv és nyereségfelosztás." },
  authorize: { title: "Éles kereskedés engedélyezése", text: "Kifejezett engedély Face ID-val; 30 napig érvényes." },
};

export default function OnboardingPage() {
  const ob = useResource(api.onboarding, ["onboarding"]);
  // undefined = not chosen yet → default to the first unfinished step.
  const [chosen, setOpen] = useState<OnboardingStepId | null | undefined>(undefined);
  const open = chosen === undefined ? (ob.data?.steps.find((s) => s.status !== "done")?.id ?? null) : chosen;

  const data = ob.data;
  return (
    <div className="animate-fade-up">
      <PageHeader title="Fiókok és élesítés" subtitle="Egyszeri beállítás. Utána a bot önállóan dolgozik; bármikor leállíthatod." />
      {ob.loading ? (
        <LoadingRows rows={5} />
      ) : ob.error ? (
        <ErrorState error={ob.error} onRetry={ob.reload} />
      ) : data ? (
        <>
          {data.ready && (
            <Card className="mb-4 border-gain/30 bg-gain/5 p-4 text-[14px]">Minden lépés kész. A rendszer élesítve van a kiválasztott brókereknél.</Card>
          )}
          <div className="space-y-2">
            {data.steps.map((s, i) => (
              <Card key={s.id} className={cn("overflow-hidden transition-colors", open === s.id && "border-line-strong")}>
                <button className="flex w-full items-center gap-3 p-4 text-left" onClick={() => setOpen(open === s.id ? null : s.id)} aria-expanded={open === s.id}>
                  <StepIcon status={s.status} n={i + 1} />
                  <div className="min-w-0 flex-1">
                    <div className="text-[15px] font-semibold">{TITLES[s.id].title}</div>
                    <div className="truncate text-[12.5px] text-muted">{s.detail || TITLES[s.id].text}</div>
                  </div>
                  {s.status === "done" && <Badge tone="gain">Kész</Badge>}
                  {s.status === "error" && <Badge tone="loss">Hiba</Badge>}
                  {s.status === "blocked" && <Badge tone="warn">Blokkolva</Badge>}
                </button>
                {open === s.id && (
                  <div className="border-t border-line p-4 animate-fade-up">
                    <StepBody id={s.id} ob={data} onChanged={ob.reload} />
                  </div>
                )}
              </Card>
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}

function StepIcon({ status, n }: { status: string; n: number }) {
  if (status === "done") return <CircleCheck size={24} className="shrink-0 text-gain" />;
  if (status === "error" || status === "blocked") return <CircleAlert size={24} className="shrink-0 text-warn" />;
  return (
    <span className="num flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-line-strong text-[12px] text-muted">
      {n}
    </span>
  );
}

function StepBody({ id, ob, onChanged }: { id: OnboardingStepId; ob: Onboarding; onChanged: () => void }) {
  switch (id) {
    case "passkey":
      return <p className="text-[13px] text-muted">Ezen az eszközön már van passkey. További eszközt a Beállításokban párosíthatsz.</p>;
    case "trading212":
      return (
        <CredentialStep
          broker="trading212"
          done={ob.steps.find((s) => s.id === "trading212")?.status === "done"}
          onChanged={onChanged}
          secret
          guide={
            <>
              Trading 212 app → Beállítások → API (Béta) → új kulcs. Csak a szükséges jogosultságokat add meg (fiók, portfólió, megbízások, metaadat,
              előzmények), és állítsd be az IP-korlátozást a szerver IP-címére. A Trading 212 API-ja nem tud pénzt kivenni.
            </>
          }
        />
      );
    case "bitvavo":
      return (
        <CredentialStep
          broker="bitvavo"
          done={ob.steps.find((s) => s.id === "bitvavo")?.status === "done"}
          onChanged={onChanged}
          secret
          operator
          attestable
          guide={
            <>
              Bitvavo → Beállítások → API → új kulcs: <b className="text-fg">csak „Olvasás” és „Kereskedés”</b>. A „Kivétel” jogosultságot{" "}
              <b className="text-loss">soha ne kapcsold be</b> — az ilyen kulcsot a rendszer elutasítja. Adj meg IP-korlátozást a szerver címére.
            </>
          }
        />
      );
    case "market_data":
      return (
        <CredentialStep
          broker="eodhd"
          done={ob.steps.find((s) => s.id === "market_data")?.status === "done"}
          onChanged={onChanged}
          guide={
            <>
              Részvény-stratégiákhoz napi árfolyamadat kell. Az EODHD ingyenes szintje (20 hívás/nap) a hosszú távú ETF-vásárláshoz elég; a
              részvénypiac pásztázásához fizetős csomag szükséges. Kulcs nélkül a részvény-bot blokkolva marad.
            </>
          }
        />
      );
    case "risk":
      return <RiskStep onChanged={onChanged} />;
    case "plan":
      return (
        <p className="text-[13px] text-muted">
          Alapértelmezés: 2026. novemberben 25 000 Ft induló tőke, 2027. áprilisig havi +50 000 Ft, 2027. májustól havi 100 000 Ft kereskedésre és
          hosszú távú befektetésre.{" "}
          <Link href="/plan/" className="font-medium text-accent">
            Terv megnyitása
          </Link>
        </p>
      );
    case "authorize":
      return <AuthorizeStep ob={ob} onChanged={onChanged} />;
  }
}

function CredentialStep({
  broker,
  done,
  onChanged,
  guide,
  secret = false,
  operator = false,
  attestable = false,
}: {
  broker: "trading212" | "bitvavo" | "eodhd";
  done: boolean;
  onChanged: () => void;
  guide: ReactNode;
  secret?: boolean;
  operator?: boolean;
  attestable?: boolean;
}) {
  const [key, setKey] = useState("");
  const [sec, setSec] = useState("");
  const [op, setOp] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [attest, setAttest] = useState(false);

  async function save() {
    setBusy(true);
    setMsg(null);
    try {
      const token = done ? await stepUp("credentials_change") : undefined;
      await api.saveCredentials(
        { broker, api_key: key.trim(), api_secret: secret ? sec.trim() : undefined, operator_id: operator ? op.trim() || undefined : undefined },
        token,
      );
      setKey("");
      setSec("");
      await api.verify(broker);
      setMsg({ ok: true, text: "Elmentve és titkosítva. Ellenőrzés folyamatban (jogosultság, kapcsolat, egyenleg)…" });
      onChanged();
    } catch (e) {
      setMsg({ ok: false, text: e instanceof ApiError ? e.message : friendlyWebAuthnError(e) });
    } finally {
      setBusy(false);
    }
  }

  async function sendAttest() {
    try {
      await api.attest(broker, await stepUp("credentials_change"));
      onChanged();
    } catch (e) {
      setMsg({ ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  }

  const valid = key.trim().length >= 8 && (!secret || sec.trim().length >= 8) && (!operator || /^\d+$/.test(op.trim()));
  return (
    <div className="space-y-4">
      <p className="text-[13px] leading-relaxed text-muted">{guide}</p>
      <form
        className="space-y-3"
        autoComplete="off"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <div>
          <Label htmlFor={`${broker}-key`}>API kulcs</Label>
          <Input id={`${broker}-key`} value={key} onChange={(e) => setKey(e.target.value)} spellCheck={false} autoCapitalize="off" />
        </div>
        {secret && (
          <div>
            <Label htmlFor={`${broker}-secret`}>API titok</Label>
            <Input id={`${broker}-secret`} type="password" value={sec} onChange={(e) => setSec(e.target.value)} autoComplete="new-password" />
          </div>
        )}
        {operator && (
          <div>
            <Label htmlFor={`${broker}-op`}>Operator ID (szám, a bot azonosítója)</Label>
            <Input id={`${broker}-op`} inputMode="numeric" value={op} onChange={(e) => setOp(e.target.value.replace(/\D/g, ""))} />
          </div>
        )}
        <div className="flex items-center gap-2 text-[12px] text-subtle">
          <Lock size={13} /> HTTPS-en jut a szerverre, ott azonnal titkosítva tárolódik; soha nem jelenik meg újra.
        </div>
        <Button type="submit" variant="accent" disabled={busy || !valid}>
          {busy ? "Mentés…" : done ? "Kulcs cseréje (Face ID)" : "Mentés és ellenőrzés"}
        </Button>
      </form>
      {attestable && (
        <label className="flex items-start gap-3 rounded-2xl border border-line p-3 text-[13px]">
          <input type="checkbox" className="mt-0.5 h-4 w-4 accent-[#a3e635]" checked={attest} onChange={(e) => setAttest(e.target.checked)} />
          <span className="flex-1 text-muted">
            Megerősítem, hogy a kulcson a „Kivétel” jogosultság ki van kapcsolva. (Csak akkor szükséges, ha az automatikus ellenőrzés nem tudta
            eldönteni.)
          </span>
          <Button size="sm" variant="secondary" disabled={!attest} onClick={sendAttest}>
            Rögzítés
          </Button>
        </label>
      )}
      {msg && <p className={`text-[13px] ${msg.ok ? "text-gain" : "text-loss"}`}>{msg.text}</p>}
    </div>
  );
}

function RiskStep({ onChanged }: { onChanged: () => void }) {
  const risk = useResource(api.risk, []);
  const [edit, setDraft] = useState<RiskConfig | null>(null);
  const draft = edit ?? risk.data?.config ?? null;
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  if (risk.loading) return <LoadingRows rows={2} />;
  if (risk.error) return <ErrorState error={risk.error} onRetry={risk.reload} />;
  if (!risk.data || !draft) return null;
  const errs = riskErrors(draft, risk.data.hard_max);
  return (
    <div>
      <RiskForm config={draft} hardMax={risk.data.hard_max} onChange={setDraft} />
      <Button
        className="mt-3"
        variant="accent"
        disabled={Object.keys(errs).length > 0}
        onClick={() =>
          api
            .onboardingRisk(draft)
            .then(() => {
              setMsg({ ok: true, text: "Mentve." });
              onChanged();
            })
            .catch((e: unknown) => setMsg({ ok: false, text: e instanceof Error ? e.message : String(e) }))
        }
      >
        Plafonok mentése
      </Button>
      {msg && <p className={`mt-2 text-[13px] ${msg.ok ? "text-gain" : "text-loss"}`}>{msg.text}</p>}
    </div>
  );
}

function AuthorizeStep({ ob, onChanged }: { ob: Onboarding; onChanged: () => void }) {
  const ready = (id: OnboardingStepId) => ob.steps.find((s) => s.id === id)?.status === "done";
  const [brokers, setBrokers] = useState<string[]>([]);
  const [ack, setAck] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const options = [
    { id: "trading212", label: "Trading 212", ok: ready("trading212") },
    { id: "bitvavo", label: "Bitvavo", ok: ready("bitvavo") },
  ];
  const prereq = ready("risk");

  async function authorize() {
    setBusy(true);
    setMsg(null);
    try {
      const token = await stepUp("live_enable");
      await api.authorize(brokers, token);
      setMsg({ ok: true, text: "Éles kereskedés engedélyezve. Az engine egyeztetés után indul." });
      onChanged();
    } catch (e) {
      setMsg({ ok: false, text: e instanceof ApiError ? e.message : friendlyWebAuthnError(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        {options.map((o) => (
          <label key={o.id} className={cn("flex items-center gap-3 rounded-2xl border border-line p-3 text-[14px]", !o.ok && "opacity-50")}>
            <input
              type="checkbox"
              className="h-4 w-4 accent-[#a3e635]"
              disabled={!o.ok}
              checked={brokers.includes(o.id)}
              onChange={(e) => setBrokers(e.target.checked ? [...brokers, o.id] : brokers.filter((b) => b !== o.id))}
            />
            <span className="flex-1 font-medium">{o.label}</span>
            {!o.ok && <span className="text-[12px] text-subtle">előbb ellenőrizd a kulcsot</span>}
          </label>
        ))}
      </div>
      <label className="flex items-start gap-3 text-[13px] leading-relaxed text-muted">
        <input type="checkbox" className="mt-1 h-4 w-4 accent-[#a3e635]" checked={ack} onChange={(e) => setAck(e.target.checked)} />
        Tudomásul veszem, hogy a bot valódi pénzzel, önállóan kereskedik; veszteség lehetséges, hozam nem garantált. A kockázatkezelő minden
        megbízást ellenőriz, és bármikor leállíthatom a Stop gombbal.
      </label>
      <Button variant="accent" size="lg" className="w-full" disabled={busy || !ack || brokers.length === 0 || !prereq} onClick={authorize}>
        <ScanFace size={18} />
        {busy ? "Engedélyezés…" : "Éles kereskedés engedélyezése"}
      </Button>
      {!prereq && (
        <p className="flex items-center gap-2 text-[12.5px] text-warn">
          <CircleDashed size={14} /> Előbb mentsd a kockázati plafonokat.
        </p>
      )}
      {msg && <p className={`text-[13px] ${msg.ok ? "text-gain" : "text-loss"}`}>{msg.text}</p>}
    </div>
  );
}
