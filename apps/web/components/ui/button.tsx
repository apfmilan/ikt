import { cva, type VariantProps } from "class-variance-authority";
import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "@/lib/utils";

export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-xl text-sm font-medium transition-[background,color,transform,opacity] duration-150 active:scale-[0.98] disabled:pointer-events-none disabled:opacity-40 select-none",
  {
    variants: {
      variant: {
        default: "bg-fg text-bg hover:bg-white/90",
        accent: "bg-accent text-black hover:bg-accent/90",
        secondary: "bg-surface-3 text-fg hover:bg-[#1d1d1d] border border-line",
        ghost: "text-muted hover:text-fg hover:bg-surface-2",
        outline: "border border-line-strong text-fg hover:bg-surface-2",
        destructive: "bg-loss text-white hover:bg-loss/90",
        "destructive-soft": "bg-loss/12 text-loss border border-loss/30 hover:bg-loss/20",
      },
      size: {
        sm: "h-9 px-3",
        md: "h-11 px-4",
        lg: "h-12 px-5 text-base",
        icon: "h-11 w-11",
        "icon-sm": "h-9 w-9",
      },
    },
    defaultVariants: { variant: "default", size: "md" },
  },
);

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { className, variant, size, type = "button", ...props },
  ref,
) {
  return <button ref={ref} type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
});
