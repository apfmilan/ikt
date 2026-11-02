"use client";

import { KeyRound, ScanFace } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { AuthCard } from "@/components/auth-card";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import { api, ApiError } from "@/lib/api";
import { useClientValue } from "@/lib/client";
import { createPasskey, deviceName, friendlyWebAuthnError, getAssertion, isWebAuthnAvailable } from "@/lib/webauthn";

export default function LoginPage() {
  const router = useRouter();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [recovery, setRecovery] = useState(false);
  const [code, setCode] = useState("");
  const supported = useClientValue(isWebAuthnAvailable, true);

  useEffect(() => {
    api
      .authStatus()
      .then((s) => {
        if (s.setup_required) router.replace(`/setup/${window.location.hash}`);
        else if (s.authenticated) router.replace("/");
      })
      .catch(() => {});
  }, [router]);

  async function login() {
    setBusy(true);
    setErr(null);
    try {
      const start = await api.loginStart();
      const credential = await getAssertion(start.options);
      await api.loginFinish(start.challenge_id, credential);
      router.replace("/");
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : friendlyWebAuthnError(e));
    } finally {
      setBusy(false);
    }
  }

  async function recover() {
    setBusy(true);
    setErr(null);
    try {
      const start = await api.recover(code.trim(), deviceName());
      const credential = await createPasskey(start.options);
      const res = await api.setupFinish(start.challenge_id, credential);
      if (res.ok) router.replace("/settings/");
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : friendlyWebAuthnError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthCard title="Trading Center" subtitle="Belépés Face ID-val vagy Windows Hello-val.">
      {!supported && (
        <p className="mb-4 rounded-2xl border border-warn/30 bg-warn/10 p-3 text-[13px] text-warn">
          Ez a böngésző nem támogatja a passkey-t. Nyisd meg Safariban (iPhone) vagy Edge-ben (Windows), HTTPS-en.
        </p>
      )}
      {!recovery ? (
        <>
          <Button size="lg" variant="accent" className="w-full" onClick={login} disabled={busy || !supported}>
            <ScanFace size={20} />
            {busy ? "Hitelesítés…" : "Belépés passkey-vel"}
          </Button>
          <button onClick={() => setRecovery(true)} className="mt-5 w-full text-center text-[13px] text-muted hover:text-fg">
            Elvesztettem az eszközöm — helyreállító kód
          </button>
        </>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void recover();
          }}
          className="space-y-4"
        >
          <div>
            <Label htmlFor="code">Helyreállító kód</Label>
            <Input id="code" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} placeholder="XXXX-XXXX-XXXX" />
          </div>
          <Button type="submit" size="lg" className="w-full" disabled={busy || code.trim().length < 8}>
            <KeyRound size={18} />
            Új passkey regisztrálása
          </Button>
          <button type="button" onClick={() => setRecovery(false)} className="w-full text-center text-[13px] text-muted hover:text-fg">
            Vissza
          </button>
        </form>
      )}
      {err && (
        <p role="alert" className="mt-4 text-center text-[13px] text-loss">
          {err}
        </p>
      )}
    </AuthCard>
  );
}
