import { Portal, Select as ArkSelect, createListCollection } from "@ark-ui/react";
import { Check, ChevronDown } from "lucide-react";
import { useMemo } from "react";

export interface SelectOption {
  label: string;
  value: string;
  description?: string;
  disabled?: boolean;
}

export interface SelectProps {
  label?: string;
  value: string;
  options: readonly SelectOption[];
  placeholder?: string;
  disabled?: boolean;
  onChange: (nextValue: string) => void;
  className?: string;
  ariaLabel?: string;
}

export function Select({
  label,
  value,
  options,
  placeholder = "—",
  disabled = false,
  onChange,
  className = "",
  ariaLabel,
}: SelectProps) {
  const collection = useMemo(
    () =>
      createListCollection({
        items: [...options],
        itemToString: (item) => item.label,
        itemToValue: (item) => item.value,
        isItemDisabled: (item) => Boolean(item.disabled),
      }),
    [options],
  );

  const selectedArray = useMemo(() => (value ? [value] : []), [value]);

  return (
    <ArkSelect.Root
      collection={collection}
      value={selectedArray}
      disabled={disabled}
      positioning={{ sameWidth: true, gutter: 4 }}
      onValueChange={(details) => {
        const next = details.value[0];
        if (next !== undefined) {
          onChange(next);
        }
      }}
      className={`w-full ${className}`}
    >
      {label ? (
        <ArkSelect.Label className="block mb-1 text-xs font-medium" style={{ color: "var(--text)" }}>
          {label}
        </ArkSelect.Label>
      ) : null}
      <ArkSelect.Control>
        <ArkSelect.Trigger
          aria-label={ariaLabel ?? label}
          className="w-full min-h-[36px] px-3 py-1.5 flex items-center justify-between gap-2 text-left text-[13px] border cursor-pointer disabled:opacity-55 disabled:cursor-not-allowed"
          style={{
            backgroundColor: "var(--bg)",
            color: "var(--text)",
            borderColor: "var(--border)",
            borderRadius: "var(--radius-md)",
          }}
        >
          <ArkSelect.ValueText placeholder={placeholder} className="truncate" />
          <ArkSelect.Indicator className="shrink-0 inline-flex items-center" style={{ color: "var(--text-muted)" }}>
            <ChevronDown size={15} />
          </ArkSelect.Indicator>
        </ArkSelect.Trigger>
      </ArkSelect.Control>
      <Portal>
        <ArkSelect.Positioner style={{ zIndex: 70 }}>
          <ArkSelect.Content
            className="max-h-64 overflow-y-auto p-1 border shadow-lg"
            style={{
              backgroundColor: "var(--bg)",
              color: "var(--text)",
              borderColor: "var(--border)",
              borderRadius: "var(--radius-md)",
            }}
          >
            {collection.items.map((item) => {
              const isSelected = item.value === value;
              return (
                <ArkSelect.Item
                  key={item.value}
                  item={item}
                  className="px-2.5 py-1.5 flex items-center justify-between gap-2 text-[13px] cursor-pointer data-[highlighted]:opacity-90"
                  style={{
                    borderRadius: "var(--radius-md)",
                    backgroundColor: isSelected ? "var(--surface-2)" : "transparent",
                    color: "var(--text)",
                  }}
                >
                  <div className="min-w-0 flex-1">
                    <ArkSelect.ItemText className="block truncate font-medium">{item.label}</ArkSelect.ItemText>
                    {item.description ? (
                      <span className="block text-[11px] truncate" style={{ color: "var(--text-muted)" }}>
                        {item.description}
                      </span>
                    ) : null}
                  </div>
                  <ArkSelect.ItemIndicator className="shrink-0 inline-flex items-center">
                    <Check size={14} />
                  </ArkSelect.ItemIndicator>
                </ArkSelect.Item>
              );
            })}
          </ArkSelect.Content>
        </ArkSelect.Positioner>
      </Portal>
    </ArkSelect.Root>
  );
}
