import { Combobox as ArkCombobox, Portal, createListCollection } from "@ark-ui/react";
import { Check, ChevronsUpDown } from "lucide-react";
import { useMemo, useState } from "react";
import type { SelectOption } from "./Select";

export interface ComboboxProps {
  label?: string;
  value: string;
  options: readonly SelectOption[];
  placeholder?: string;
  disabled?: boolean;
  onChange: (nextValue: string) => void;
  className?: string;
}

export function Combobox({
  label,
  value,
  options,
  placeholder = "",
  disabled = false,
  onChange,
  className = "",
}: ComboboxProps) {
  const [query, setQuery] = useState("");

  const filteredItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      return [...options];
    }
    return options.filter(
      (opt) => opt.label.toLowerCase().includes(q) || opt.value.toLowerCase().includes(q),
    );
  }, [options, query]);

  const collection = useMemo(
    () =>
      createListCollection({
        items: filteredItems,
        itemToString: (item) => item.label,
        itemToValue: (item) => item.value,
      }),
    [filteredItems],
  );

  return (
    <ArkCombobox.Root
      collection={collection}
      value={value ? [value] : []}
      disabled={disabled}
      positioning={{ sameWidth: true, gutter: 4 }}
      onInputValueChange={(details) => setQuery(details.inputValue)}
      onValueChange={(details) => {
        const next = details.value[0];
        if (next !== undefined) {
          onChange(next);
        }
      }}
      className={`w-full ${className}`}
    >
      {label ? (
        <ArkCombobox.Label className="block mb-1 text-xs font-medium" style={{ color: "var(--text)" }}>
          {label}
        </ArkCombobox.Label>
      ) : null}
      <ArkCombobox.Control className="relative flex items-center">
        <ArkCombobox.Input
          placeholder={placeholder}
          className="park-input pr-8"
        />
        <ArkCombobox.Trigger
          aria-label={label ?? "展开选项"}
          className="absolute right-2 inline-flex items-center justify-center p-1 cursor-pointer"
          style={{ color: "var(--text-muted)" }}
        >
          <ChevronsUpDown size={14} />
        </ArkCombobox.Trigger>
      </ArkCombobox.Control>
      <Portal>
        <ArkCombobox.Positioner style={{ zIndex: 70 }}>
          <ArkCombobox.Content
            className="max-h-60 overflow-y-auto p-1 border shadow-lg"
            style={{
              backgroundColor: "var(--bg)",
              color: "var(--text)",
              borderColor: "var(--border)",
              borderRadius: "var(--radius-md)",
            }}
          >
            {collection.items.length === 0 ? (
              <div className="px-2.5 py-2 text-xs" style={{ color: "var(--text-muted)" }}>
                无匹配项
              </div>
            ) : (
              collection.items.map((item) => (
                <ArkCombobox.Item
                  key={item.value}
                  item={item}
                  className="px-2.5 py-1.5 flex items-center justify-between gap-2 text-[13px] cursor-pointer"
                  style={{
                    borderRadius: "var(--radius-md)",
                    backgroundColor: item.value === value ? "var(--surface-2)" : "transparent",
                    color: "var(--text)",
                  }}
                >
                  <ArkCombobox.ItemText className="truncate">{item.label}</ArkCombobox.ItemText>
                  <ArkCombobox.ItemIndicator>
                    <Check size={14} />
                  </ArkCombobox.ItemIndicator>
                </ArkCombobox.Item>
              ))
            )}
          </ArkCombobox.Content>
        </ArkCombobox.Positioner>
      </Portal>
    </ArkCombobox.Root>
  );
}
