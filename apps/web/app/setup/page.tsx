"use client";

import { Check, Copy, ScanFace } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { AuthCard } from "@/components/auth-card";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import { api, ApiError } from "@/lib/api";
import { useClientValue, useHashParam } from "@/lib/client";
import { createPasskey, deviceName, friendlyWebAuthnError, isWebAuthnAvailable } from "@/lib/webauthn";

/** First-time setup. The one-time setup token arrives in the URL fragment (#token=…) so it is
 *  never sent to the server in a request line or logged; it is posted once in a JSON body. */
export default function SetupPage() {
  const router = useRouter();
  const hashToken = useHashParam("token");
  const hashPair = useHashParam("pair");
  const defaultName = useClientValue(deviceName, "Eszköz");
  const [tokenEdit, setToken] = useState<string | null>(null);
  const [pairEdit, setPairing] = useState<string | null>(null);
  const [nameEdit, setName] = useState<string | null>(null);
  const token = tokenEdit ?? hashToken;
  const pairing = pairEdit ?? hashPair;
  const name = nameEdit ?? defaultName;
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);
  const [copied, setCopied] = useState(false);
  const [modeOverride, setMode] = useState<"setup" | "pair" | null>(null);
  const mode = modeOverride ?? (hashPair ? "pair" : "setup");
  const supported = useClientValue(isWebAuthnAvailable, true);

  useEffect(() => {
    const p = new URLSearchParams(window.location.hash.slice(1)).get("pair");
    api
      .authStatus()
      .then((s) => {
        if (!s.setup_required && !p && s.authenticated) router.replace("/");
        if (!s.setup_required && !p && !s.authenticated) setMode("pair");
      })
      .catch(() => {});
  }, [router]);

  async function register() {
    setBusy(true);
    setErr(null);
    try {
      const start = mode === "setup" ? await api.setupStart(token.trim(), name.trim() || "Eszköz") : await api.pairStart(pairing.trim(), name.trim() || "Eszköz");
      const credential = await createPasskey(start.options);
      const res = await api.setupFinish(start.challenge_id, credential);
      // The single-use token/pairing code is consumed; drop it from the URL fragment.
      history.replaceState(null, "", window.location.pathname);
      if (res.recovery_codes?.length) setCodes(res.recovery_codes);
      else router.replace("/onboarding/");
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : friendlyWebAuthnError(e));
    } finally {
      setBusy(false);
    }
  }

  if (codes) {
    return (
      <AuthCard title="Helyreállító kódok" subtitle="Írd le vagy mentsd jelszókezelőbe. Csak most látod őket, mindegyik egyszer használható.">
        <div className="grid grid-cols-2 gap-2 rounded-2xl border border-line bg-surface p-4 font-mono text-[14px]">
          {codes.map((c) => (
            <span key={c} className="num tracking-wider">
              {c}
            </span>
          ))}
        </div>
        <Button
          variant="secondary"
          className="mt-3 w-full"
          onClick={() => {
            void navigator.clipboard?.writeText(codes.join("\n")).then(() => setCopied(true));
          }}
        >
          {copied ? <Check size={16} /> : <Copy size={16} />}
          {copied ? "Másolva" : "Másolás"}
        </Button>
        <Button variant="accent" size="lg" className="mt-3 w-full" onClick={() => router.replace("/onboarding/")}>
          Elmentettem, tovább
        </Button>
      </AuthCard>
    );
  }

  return (
    <AuthCard
      title={mode === "setup" ? "Első beállítás" : "Új eszköz párosítása"}
      subtitle={
        mode === "setup"
          ? "Hozd létre a passkey-t ezen az eszközön. A beállító kódot a szerver adja: tc setup-token."
          : "Add meg a már belépett eszközön kapott párosító kódot."
      }
    >
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void register();
        }}
      >
        {mode === "setup" ? (
          <div>
            <Label htmlFor="token">Beállító kód</Label>
            <Input id="token" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" spellCheck={false} />
          </div>
        ) : (
          <div>
            <Label htmlFor="pair">Párosító kód</Label>
            <Input id="pair" value={pairing} onChange={(e) => setPairing(e.target.value)} autoComplete="one-time-code" inputMode="numeric" />
          </div>
        )}
        <div>
          <Label htmlFor="name">Eszköz neve</Label>
          <Input id="name" value={name} onChange={(e) => setName(e.target.value)} maxLength={40} />
        </div>
        <Button
          type="submit"
          size="lg"
          variant="accent"
          className="w-full"
          disabled={busy || !supported || (mode === "setup" ? token.trim().length < 16 : pairing.trim().length < 6)}
        >
          <ScanFace size={20} />
          {busy ? "Regisztráció…" : "Passkey létrehozása"}
        </Button>
      </form>
      {err && (
        <p role="alert" className="mt-4 text-center text-[13px] text-loss">
          {err}
        </p>
      )}
    </AuthCard>
  );
}
