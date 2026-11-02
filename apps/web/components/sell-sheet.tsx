"use client";

import { ScanFace } from "lucide-react";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";
import { huf, price, qty } from "@/lib/format";
import type { PositionRow } from "@/lib/types";
import { friendlyWebAuthnError, stepUp } from "@/lib/webauthn";
import { Segmented } from "./ui/segmented";
import { Button } from "./ui/button";
import { Input, Label } from "./ui/input";
import { Sheet } from "./ui/sheet";

/** Manual sell: confirm sheet → passkey step-up → POST /positions/sell. The engine still
 *  routes the order through the risk engine (exit rules) before sending. */
export function SellSheet({ position, onClose, onDone }: { position: PositionRow; onClose: () => void; onDone: () => void }) {
  const [mode, setMode] = useState<"all" | "part">("all");
  const [amount, setAmount] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [sent, setSent] = useState(false);

  const partValid = /^\d+([.,]\d+)?$/.test(amount.trim()) && Number(amount.replace(",", ".")) > 0 && Number(amount.replace(",", ".")) <= Number(position.quantity);

  async function confirm() {
    setBusy(true);
    setErr(null);
    try {
      const token = await stepUp("manual_sell");
      await api.sell(position.instrument, mode === "all" ? "all" : amount.trim().replace(",", "."), token);
      setSent(true);
      onDone();
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : friendlyWebAuthnError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Sheet open onClose={onClose} title={`${position.symbol} eladása`}>
      {sent ? (
        <div className="space-y-4">
          <p className="text-[14px]">Az eladási utasítás elküldve. A kockázatkezelő ellenőrzése után kerül a brókerhez; az eredményt a megbízásoknál látod.</p>
          <Button className="w-full" onClick={onClose}>
            Rendben
          </Button>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="grid grid-cols-3 gap-2 rounded-2xl bg-surface p-3 text-[13px]">
            <div>
              <div className="text-subtle">Mennyiség</div>
              <div className="num font-medium">{qty(position.quantity)}</div>
            </div>
            <div>
              <div className="text-subtle">Árfolyam</div>
              <div className="num font-medium">{price(position.last_price, position.currency)}</div>
            </div>
            <div>
              <div className="text-subtle">Érték</div>
              <div className="num font-medium">{huf(position.value_huf)}</div>
            </div>
          </div>
          <Segmented
            value={mode}
            onChange={setMode}
            options={[
              { value: "all", label: "Teljes pozíció" },
              { value: "part", label: "Részleges" },
            ]}
          />
          {mode === "part" && (
            <div>
              <Label htmlFor="sell-qty">Eladandó mennyiség</Label>
              <Input id="sell-qty" inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} placeholder={position.quantity} />
            </div>
          )}
          <p className="text-[12.5px] leading-snug text-muted">
            Piaci áron történik (kriptónál ársávval védett limit). Zárt tőzsdén a megbízás nyitáskor teljesül; a stop addig a brókernél marad.
          </p>
          <Button variant="destructive" size="lg" className="w-full" disabled={busy || (mode === "part" && !partValid)} onClick={confirm}>
            <ScanFace size={18} />
            {busy ? "Megerősítés…" : "Eladás megerősítése"}
          </Button>
          {err && (
            <p role="alert" className="text-[13px] text-loss">
              {err}
            </p>
          )}
        </div>
      )}
    </Sheet>
  );
}
