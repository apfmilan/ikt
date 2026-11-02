// Minimal WebAuthn helpers (no dependency). Converts webauthn-rs JSON options (base64url)
// to the binary form navigator.credentials expects, and serialises the result back.

import { api } from "./api";
import type { StepUpAction } from "./types";

export function b64urlToBuf(s: string): ArrayBuffer {
  const pad = "=".repeat((4 - (s.length % 4)) % 4);
  const b64 = (s + pad).replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

export function bufToB64url(buf: ArrayBuffer | ArrayBufferView | null | undefined): string | null {
  if (!buf) return null;
  const bytes = buf instanceof ArrayBuffer ? new Uint8Array(buf) : new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i] ?? 0);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

type Json = Record<string, unknown>;

function unwrap(options: unknown): Json {
  const o = (options ?? {}) as Json;
  return (o.publicKey as Json | undefined) ?? o;
}

function convertDescriptors(list: unknown): PublicKeyCredentialDescriptor[] | undefined {
  if (!Array.isArray(list)) return undefined;
  return list.map((d: Json) => ({
    type: "public-key" as const,
    id: b64urlToBuf(String(d.id)),
    transports: d.transports as AuthenticatorTransport[] | undefined,
  }));
}

export function isWebAuthnAvailable(): boolean {
  return typeof window !== "undefined" && typeof window.PublicKeyCredential !== "undefined";
}

export async function createPasskey(options: unknown): Promise<Json> {
  const pk = unwrap(options);
  const user = pk.user as Json;
  const publicKey: PublicKeyCredentialCreationOptions = {
    ...(pk as unknown as PublicKeyCredentialCreationOptions),
    challenge: b64urlToBuf(String(pk.challenge)),
    user: { ...(user as unknown as PublicKeyCredentialUserEntity), id: b64urlToBuf(String(user.id)) },
    excludeCredentials: convertDescriptors(pk.excludeCredentials),
  };
  const cred = (await navigator.credentials.create({ publicKey })) as PublicKeyCredential | null;
  if (!cred) throw new Error("A passkey létrehozása megszakadt.");
  const r = cred.response as AuthenticatorAttestationResponse;
  return {
    id: cred.id,
    rawId: bufToB64url(cred.rawId),
    type: cred.type,
    response: {
      attestationObject: bufToB64url(r.attestationObject),
      clientDataJSON: bufToB64url(r.clientDataJSON),
      transports: typeof r.getTransports === "function" ? r.getTransports() : undefined,
    },
    extensions: cred.getClientExtensionResults(),
  };
}

export async function getAssertion(options: unknown): Promise<Json> {
  const pk = unwrap(options);
  const publicKey: PublicKeyCredentialRequestOptions = {
    ...(pk as unknown as PublicKeyCredentialRequestOptions),
    challenge: b64urlToBuf(String(pk.challenge)),
    allowCredentials: convertDescriptors(pk.allowCredentials),
  };
  const cred = (await navigator.credentials.get({ publicKey })) as PublicKeyCredential | null;
  if (!cred) throw new Error("A hitelesítés megszakadt.");
  const r = cred.response as AuthenticatorAssertionResponse;
  return {
    id: cred.id,
    rawId: bufToB64url(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: bufToB64url(r.authenticatorData),
      clientDataJSON: bufToB64url(r.clientDataJSON),
      signature: bufToB64url(r.signature),
      userHandle: bufToB64url(r.userHandle),
    },
    extensions: cred.getClientExtensionResults(),
  };
}

/** Step-up re-authentication bound to one action. Returns a single-use token. */
export async function stepUp(action: StepUpAction): Promise<string> {
  const start = await api.stepupStart(action);
  const credential = await getAssertion(start.options);
  const res = await api.stepupFinish(start.challenge_id, action, credential);
  return res.stepup_token;
}

export function friendlyWebAuthnError(e: unknown): string {
  if (e instanceof DOMException) {
    if (e.name === "NotAllowedError") return "A hitelesítést megszakítottad vagy lejárt.";
    if (e.name === "InvalidStateError") return "Ez az eszköz már regisztrálva van.";
    if (e.name === "SecurityError") return "Biztonsági hiba: csak HTTPS-en, a beállított domainen működik.";
  }
  return e instanceof Error ? e.message : "Ismeretlen hiba.";
}

export function deviceName(): string {
  if (typeof navigator === "undefined") return "Eszköz";
  const ua = navigator.userAgent;
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua)) return "iPad";
  if (/Windows/.test(ua)) return "Windows";
  if (/Mac OS X/.test(ua)) return "Mac";
  if (/Android/.test(ua)) return "Android";
  return "Böngésző";
}
