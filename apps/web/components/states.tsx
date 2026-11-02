import { CircleX, Inbox, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { ApiError } from "@/lib/api";
import { Button } from "./ui/button";
import { Skeleton } from "./ui/skeleton";

export function EmptyState({ title, hint, action }: { title: string; hint?: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 rounded-[var(--radius-card)] border border-dashed border-line px-6 py-10 text-center">
      <Inbox size={22} className="text-subtle" aria-hidden />
      <p className="text-sm font-medium text-fg">{title}</p>
      {hint && <p className="max-w-xs text-[13px] text-muted">{hint}</p>}
      {action}
    </div>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const msg = error instanceof ApiError ? `${error.message}${error.code ? ` (${error.code})` : ""}` : error instanceof Error ? error.message : String(error);
  return (
    <div role="alert" className="flex items-start gap-3 rounded-[var(--radius-card)] border border-loss/25 bg-loss/5 p-4">
      <CircleX size={18} className="mt-0.5 shrink-0 text-loss" aria-hidden />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-fg">Nem sikerült betölteni</p>
        <p className="mt-0.5 break-words text-[13px] text-muted">{msg}</p>
      </div>
      {onRetry && (
        <Button variant="ghost" size="icon-sm" onClick={onRetry} aria-label="Újra">
          <RefreshCw size={16} />
        </Button>
      )}
    </div>
  );
}

export function LoadingRows({ rows = 3 }: { rows?: number }) {
  return (
    <div className="space-y-2" aria-busy="true" aria-label="Betöltés">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className="h-16 w-full rounded-2xl" />
      ))}
    </div>
  );
}
