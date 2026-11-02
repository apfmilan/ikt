import { cva, type VariantProps } from "class-variance-authority";
import type { HTMLAttributes } from "react";
import { cn } from "@/lib/utils";

const badgeVariants = cva("inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium leading-5", {
  variants: {
    tone: {
      neutral: "bg-surface-3 text-muted border border-line",
      gain: "bg-gain/12 text-gain",
      loss: "bg-loss/12 text-loss",
      warn: "bg-warn/12 text-warn",
      accent: "bg-accent/12 text-accent",
      forecast: "bg-forecast/10 text-forecast border border-forecast/25",
      realized: "bg-fg/8 text-fg border border-line-strong",
    },
  },
  defaultVariants: { tone: "neutral" },
});

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement>, VariantProps<typeof badgeVariants> {}

export function Badge({ className, tone, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ tone }), className)} {...props} />;
}

export function Dot({ className }: { className?: string }) {
  return <span aria-hidden className={cn("inline-block h-1.5 w-1.5 rounded-full bg-current", className)} />;
}
