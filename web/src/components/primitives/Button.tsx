import { forwardRef, type ButtonHTMLAttributes, type CSSProperties, type ReactNode } from "react";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, children, className = "", style, disabled, type = "button", ...rest },
  ref,
) {
  const variantStyle: CSSProperties =
    variant === "primary"
      ? {
          backgroundColor: "var(--accent)",
          color: "var(--accent-fg)",
          borderColor: "var(--accent)",
        }
      : variant === "danger"
        ? {
            backgroundColor: "var(--danger)",
            color: "var(--danger-fg)",
            borderColor: "var(--danger)",
          }
        : variant === "ghost"
          ? {
              backgroundColor: "transparent",
              color: "var(--text)",
              borderColor: "transparent",
            }
          : {
              backgroundColor: "var(--surface)",
              color: "var(--text)",
              borderColor: "var(--border)",
            };

  const sizeClass = size === "sm" ? "min-h-[30px] px-2.5 py-1 text-xs" : "min-h-[36px] px-3.5 py-1.5 text-[13px]";

  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled}
      data-park-button={variant}
      className={`inline-flex items-center justify-center gap-1.5 font-medium border transition-opacity select-none whitespace-nowrap ${
        disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer hover:opacity-90"
      } ${sizeClass} ${className}`}
      style={{
        borderRadius: "var(--radius-md)",
        ...variantStyle,
        ...style,
      }}
      {...rest}
    >
      {icon ? <span className="inline-flex shrink-0 items-center">{icon}</span> : null}
      {children}
    </button>
  );
});
