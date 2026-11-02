"use client";

import { cn } from "@/lib/utils";

export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-7 w-12 shrink-0 rounded-full border transition-colors duration-200 disabled:opacity-40",
        checked ? "border-accent/50 bg-accent" : "border-line-strong bg-surface-3",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 left-0.5 h-[22px] w-[22px] rounded-full shadow transition-transform duration-200",
          checked ? "translate-x-5 bg-black" : "translate-x-0 bg-fg",
        )}
      />
    </button>
  );
}
