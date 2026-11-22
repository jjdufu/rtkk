import { Checkbox as ArkCheckbox } from "@ark-ui/react";
import { Check, Minus } from "lucide-react";

export type CheckboxCheckedState = boolean | "indeterminate";

export interface CheckboxProps {
  checked: CheckboxCheckedState;
  onChange: (nextChecked: boolean) => void;
  label?: string;
  ariaLabel?: string;
  disabled?: boolean;
}

export function Checkbox({ checked, onChange, label, ariaLabel, disabled = false }: CheckboxProps) {
  const isActive = checked === true || checked === "indeterminate";

  return (
    <ArkCheckbox.Root
      checked={checked}
      disabled={disabled}
      onCheckedChange={(details) => {
        onChange(details.checked === true);
      }}
      className={`inline-flex items-center gap-2 select-none ${
        disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
      }`}
    >
      <ArkCheckbox.Control
        aria-label={ariaLabel ?? label}
        className="w-4 h-4 inline-flex items-center justify-center border shrink-0 transition-colors"
        style={{
          borderRadius: "4px",
          backgroundColor: isActive ? "var(--accent)" : "var(--bg)",
          borderColor: isActive ? "var(--accent)" : "var(--border)",
          color: isActive ? "var(--accent-fg)" : "var(--text)",
        }}
      >
        <ArkCheckbox.Indicator>
          <Check size={12} strokeWidth={3} />
        </ArkCheckbox.Indicator>
        <ArkCheckbox.Indicator indeterminate>
          <Minus size={12} strokeWidth={3} />
        </ArkCheckbox.Indicator>
      </ArkCheckbox.Control>
      {label ? (
        <ArkCheckbox.Label className="text-[13px]" style={{ color: "var(--text)" }}>
          {label}
        </ArkCheckbox.Label>
      ) : null}
      <ArkCheckbox.HiddenInput />
    </ArkCheckbox.Root>
  );
}
