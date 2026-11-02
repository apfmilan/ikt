import type { ReactNode } from "react";

export function AuthCard({ title, subtitle, children }: { title: string; subtitle?: string; children: ReactNode }) {
  return (
    <div className="pt-safe pb-safe flex min-h-dvh items-center justify-center px-5">
      <div className="w-full max-w-sm animate-fade-up">
        <div className="mb-8 flex flex-col items-center text-center">
          <svg width="64" height="64" viewBox="0 0 64 64" aria-hidden className="mb-5">
            <rect width="64" height="64" rx="16" fill="#0f0f0f" stroke="#262626" />
            <path d="M14 42 L26 30 L34 37 L50 20" fill="none" stroke="#a3e635" strokeWidth="5" strokeLinecap="round" strokeLinejoin="round" />
            <circle cx="50" cy="20" r="4" fill="#a3e635" />
          </svg>
          <h1 className="text-[26px] font-semibold tracking-tight">{title}</h1>
          {subtitle && <p className="mt-2 text-[14px] leading-relaxed text-muted">{subtitle}</p>}
        </div>
        {children}
      </div>
    </div>
  );
}
