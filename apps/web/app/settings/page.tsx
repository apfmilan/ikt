"use client";

import { Bell, BellOff, LogOut, Monitor, Plus, Smartphone, Trash } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { PageHeader, Section } from "@/components/page-header";
import { ErrorState, LoadingRows } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api, ApiError } from "@/lib/api";
import { ago, dateTime } from "@/lib/format";
import { useClientValue } from "@/lib/client";
import { useResource } from "@/lib/live";
import type { NotificationSettings, PairingCode } from "@/lib/types";
import { b64urlToBuf, deviceName, friendlyWebAuthnError, stepUp } from "@/lib/webauthn";

const TOGGLES: { key: keyof Omit<NotificationSettings, "daily_summary_time">; label: string; hint: string }[] = [
  { key: "trades", label: "Vétel / eladás", hint: "Minden teljesült megbízás." },
  { key: "market_open", label: "Tőzsdenyitás", hint: "Amikor a figyelt piac nyit." },
  { key: "errors", label: "Hibák", hint: "Kapcsolati vagy egyeztetési hiba." },
  { key: "critical_risk", label: "Kritikus kockázat", hint: "Vészleállítás, limit elérése." },
  { key: "daily_summary", label: "Napi összefoglaló", hint: "„💰 Mai eredmény: +1 000 Ft” a tényleges nettó eredményből." },
];

export default function SettingsPage() {
  const router = useRouter();
  const settings = useResource(api.notificationSettings, []);
  const devices = useResource(api.devices, []);
  const [edit, setDraft] = useState<NotificationSettings | null>(null);
  const draft = edit ?? settings.data ?? null;
  const env = useClientValue(pushEnvironment, "loading");
  const [subscribed, setSubscribed] = useState<boolean | null>(null);
  const pushState = env === "ok" ? (subscribed ? "on" : "off") : env;
  const [pushMsg, setPushMsg] = useState<string | null>(null);
  const [pairing, setPairing] = useState<PairingCode | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (env !== "ok") return;
    let alive = true;
    navigator.serviceWorker
      .getRegistration()
      .then((reg) => reg?.pushManager.getSubscription())
      .then((sub) => alive && setSubscribed(Boolean(sub)))
      .catch(() => alive && setSubscribed(false));
    return () => {
      alive = false;
    };
  }, [env]);

  async function saveSettings(next: NotificationSettings) {
    setDraft(next);
    try {
      await api.updateNotificationSettings(next);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      settings.reload();
    }
  }

  async function enablePush() {
    setPushMsg(null);
    try {
      // Permission prompt only in response to this tap (required by iOS).
      const perm = await Notification.requestPermission();
      if (perm !== "granted") {
        setSubscribed(false);
        return;
      }
      const reg = (await navigator.serviceWorker.getRegistration()) ?? (await navigator.serviceWorker.register("/sw.js"));
      await navigator.serviceWorker.ready;
      const { public_key } = await api.vapid();
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64urlToBuf(public_key) });
      const json = sub.toJSON();
      if (!json.endpoint || !json.keys?.p256dh || !json.keys.auth) throw new Error("Hiányos push-előfizetés.");
      await api.pushSubscribe({ endpoint: json.endpoint, keys: { p256dh: json.keys.p256dh, auth: json.keys.auth }, device_name: deviceName() });
      setSubscribed(true);
    } catch (e) {
      setPushMsg(e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e));
    }
  }

  async function disablePush() {
    const reg = await navigator.serviceWorker.getRegistration();
    const sub = await reg?.pushManager.getSubscription();
    if (sub) {
      await api.pushUnsubscribe(sub.endpoint).catch(() => {});
      await sub.unsubscribe();
    }
    setSubscribed(false);
  }

  async function removeDevice(id: string) {
    setErr(null);
    try {
      const token = await stepUp("device_remove");
      await api.removeDevice(id, token);
      devices.reload();
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : friendlyWebAuthnError(e));
    }
  }

  return (
    <div className="animate-fade-up">
      <PageHeader title="Beállítások" />

      <Section title="Push értesítések">
        <Card className="p-5">
          <div className="flex items-center gap-3">
            {pushState === "on" ? <Bell size={18} className="text-accent" /> : <BellOff size={18} className="text-muted" />}
            <div className="flex-1 text-[14px]">
              {pushState === "loading" && "…"}
              {pushState === "on" && "Ezen az eszközön bekapcsolva."}
              {pushState === "off" && "Ezen az eszközön kikapcsolva."}
              {pushState === "denied" && "Az értesítések tiltva vannak a rendszerbeállításokban."}
              {pushState === "needs-install" && "iPhone-on előbb add hozzá a Főképernyőhöz (Megosztás → Főképernyőhöz adás), majd onnan nyisd meg."}
              {pushState === "unsupported" && "Ez a böngésző nem támogatja a Web Push-t."}
            </div>
            {pushState === "off" && (
              <Button size="sm" variant="accent" onClick={enablePush}>
                Bekapcsolás
              </Button>
            )}
            {pushState === "on" && (
              <Button size="sm" variant="secondary" onClick={disablePush}>
                Kikapcsolás
              </Button>
            )}
          </div>
          {pushMsg && <p className="mt-2 text-[13px] text-loss">{pushMsg}</p>}
        </Card>

        {settings.loading ? (
          <div className="mt-3">
            <LoadingRows rows={2} />
          </div>
        ) : settings.error ? (
          <div className="mt-3">
            <ErrorState error={settings.error} onRetry={settings.reload} />
          </div>
        ) : draft ? (
          <Card className="mt-3 divide-y divide-line px-5">
            {TOGGLES.map((t) => (
              <div key={t.key} className="flex items-center gap-4 py-3.5">
                <div className="flex-1">
                  <div className="text-[14px] font-medium">{t.label}</div>
                  <div className="text-[12px] text-subtle">{t.hint}</div>
                </div>
                <Switch label={t.label} checked={draft[t.key]} onChange={(v) => saveSettings({ ...draft, [t.key]: v })} />
              </div>
            ))}
            <div className="flex items-center gap-4 py-3.5">
              <div className="flex-1 text-[14px] font-medium">Összefoglaló időpontja</div>
              <Input
                type="time"
                className="h-10 w-28"
                value={draft.daily_summary_time}
                onChange={(e) => saveSettings({ ...draft, daily_summary_time: e.target.value })}
              />
            </div>
          </Card>
        ) : null}
      </Section>

      <Section
        title="Eszközök (passkey)"
        action={
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              api
                .pair()
                .then(setPairing)
                .catch((e: unknown) => setErr(e instanceof Error ? e.message : String(e)))
            }
          >
            <Plus size={15} /> Új eszköz
          </Button>
        }
      >
        {pairing && (
          <Card className="mb-3 p-4 text-[13px]">
            <div className="text-muted">Az új eszközön nyisd meg a /setup/ oldalt, és add meg:</div>
            <div className="num mt-2 text-[28px] font-semibold tracking-[0.2em]">{pairing.pairing_code}</div>
            <div className="mt-1 text-subtle">Lejár: {dateTime(pairing.expires_at)}</div>
          </Card>
        )}
        {devices.loading ? (
          <LoadingRows rows={2} />
        ) : devices.error ? (
          <ErrorState error={devices.error} onRetry={devices.reload} />
        ) : (
          <Card className="divide-y divide-line">
            {(devices.data ?? []).map((d) => (
              <div key={d.id} className="flex items-center gap-3 p-4">
                {/Windows|Mac/.test(d.name) ? <Monitor size={18} className="text-muted" /> : <Smartphone size={18} className="text-muted" />}
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] font-medium">{d.name}</div>
                  <div className="text-[12px] text-subtle">
                    Hozzáadva {dateTime(d.created_at)} · utoljára {d.last_used_at ? ago(d.last_used_at) : "–"}
                  </div>
                </div>
                <Button size="icon-sm" variant="ghost" aria-label={`${d.name} eltávolítása`} onClick={() => removeDevice(d.id)}>
                  <Trash size={16} />
                </Button>
              </div>
            ))}
          </Card>
        )}
      </Section>

      {err && <p className="mt-3 text-[13px] text-loss">{err}</p>}

      <Section title="Munkamenet">
        <Button
          variant="secondary"
          onClick={() =>
            api
              .logout()
              .catch(() => {})
              .finally(() => router.replace("/login/"))
          }
        >
          <LogOut size={16} /> Kijelentkezés
        </Button>
      </Section>
    </div>
  );
}

/** Synchronous push capability check (stable string snapshot for useSyncExternalStore). */
function pushEnvironment(): "unsupported" | "needs-install" | "denied" | "ok" {
  const supported = "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
  if (!supported) {
    const ios = /iPhone|iPad/.test(navigator.userAgent);
    const standalone = window.matchMedia("(display-mode: standalone)").matches;
    return ios && !standalone ? "needs-install" : "unsupported";
  }
  return Notification.permission === "denied" ? "denied" : "ok";
}
