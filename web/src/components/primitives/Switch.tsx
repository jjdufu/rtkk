import { Switch as ArkSwitch } from "@ark-ui/react";

export interface SwitchProps {
  checked: boolean;
  onChange: (nextChecked: boolean) => void;
  label?: string;
  description?: string;
  disabled?: boolean;
  ariaLabel?: string;
}

export function Switch({
  checked,
  onChange,
  label,
  description,
  disabled = false,
  ariaLabel,
}: SwitchProps) {
  return (
    <ArkSwitch.Root
      checked={checked}
      disabled={disabled}
      onCheckedChange={(details) => onChange(details.checked)}
      className={`inline-flex items-center justify-between gap-3 ${
        disabled ? "opacity-55 cursor-not-allowed" : "cursor-pointer"
      }`}
    >
      {label || description ? (
        <div className="min-w-0">
          {label ? (
            <ArkSwitch.Label className="block text-[13px] font-medium" style={{ color: "var(--text)" }}>
              {label}
            </ArkSwitch.Label>
          ) : null}
          {description ? (
            <span className="block text-xs mt-0.5" style={{ color: "var(--text-muted)" }}>
              {description}
            </span>
          ) : null}
        </div>
      ) : null}
      <ArkSwitch.Control
        aria-label={ariaLabel ?? label}
        className="relative inline-flex h-5 w-9 shrink-0 items-center border transition-colors p-0.5"
        style={{
          borderRadius: "99px",
          backgroundColor: checked ? "var(--accent)" : "var(--surface-2)",
          borderColor: checked ? "var(--accent)" : "var(--border)",
        }}
      >
        <ArkSwitch.Thumb
          className="block h-3.5 w-3.5 transition-transform"
          style={{
            borderRadius: "99px",
            backgroundColor: checked ? "var(--accent-fg)" : "var(--text-muted)",
            transform: checked ? "translateX(16px)" : "translateX(0px)",
          }}
        />
      </ArkSwitch.Control>
      <ArkSwitch.HiddenInput />
    </ArkSwitch.Root>
  );
}
