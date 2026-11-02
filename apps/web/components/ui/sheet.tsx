"use client";

import { X } from "lucide-react";
import { useEffect, useId, useRef, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/** Bottom sheet on mobile, centred dialog on desktop. Esc / backdrop closes. */
export function Sheet({
  open,
  onClose,
  title,
  children,
  className,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  className?: string;
}) {
  const id = useId();
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    document.body.style.overflow = "hidden";
    panel.current?.focus();
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = "";
      prev?.focus?.();
    };
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center sm:items-center">
      <div className="absolute inset-0 bg-black/70 backdrop-blur-sm animate-fade-up" onClick={onClose} aria-hidden />
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        tabIndex={-1}
        className={cn(
          "relative z-10 w-full max-w-md rounded-t-3xl border border-line bg-surface-2 p-5 pb-[calc(1.25rem+var(--safe-bottom))] outline-none",
          "animate-sheet-up sm:rounded-3xl sm:animate-fade-up sm:pb-5",
          className,
        )}
      >
        <div className="mx-auto mb-4 h-1 w-10 rounded-full bg-line-strong sm:hidden" aria-hidden />
        <div className="mb-4 flex items-center justify-between">
          <h2 id={id} className="text-lg font-semibold tracking-tight">
            {title}
          </h2>
          <button onClick={onClose} aria-label="Bezárás" className="rounded-full p-2 text-muted hover:bg-surface-3 hover:text-fg">
            <X size={18} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
