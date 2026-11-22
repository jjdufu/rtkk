import { Tabs as ArkTabs } from "@ark-ui/react";
import type { ReactNode } from "react";

export interface TabItem {
  value: string;
  label: string;
  badge?: string | number;
  content: ReactNode;
}

export interface TabsProps {
  value: string;
  onChange: (nextValue: string) => void;
  items: readonly TabItem[];
  className?: string;
}

export function Tabs({ value, onChange, items, className = "" }: TabsProps) {
  return (
    <ArkTabs.Root
      value={value}
      onValueChange={(details) => onChange(details.value)}
      className={className}
    >
      <ArkTabs.List
        className="flex flex-wrap items-center gap-1 p-1 border mb-4"
        style={{
          backgroundColor: "var(--surface)",
          borderColor: "var(--border)",
          borderRadius: "var(--radius-md)",
        }}
      >
        {items.map((tab) => {
          const active = tab.value === value;
          return (
            <ArkTabs.Trigger
              key={tab.value}
              value={tab.value}
              className="px-3 py-1.5 text-[13px] font-medium inline-flex items-center gap-1.5 cursor-pointer transition-colors"
              style={{
                borderRadius: "var(--radius-md)",
                backgroundColor: active ? "var(--accent)" : "transparent",
                color: active ? "var(--accent-fg)" : "var(--text)",
              }}
            >
              <span>{tab.label}</span>
              {tab.badge !== undefined ? (
                <span
                  className="tabular-nums text-[11px] px-1.5 py-0.5"
                  style={{
                    borderRadius: "var(--radius-md)",
                    backgroundColor: active ? "rgba(127, 127, 127, 0.25)" : "var(--surface-2)",
                    color: active ? "var(--accent-fg)" : "var(--text-muted)",
                  }}
                >
                  {tab.badge}
                </span>
              ) : null}
            </ArkTabs.Trigger>
          );
        })}
      </ArkTabs.List>

      {items.map((tab) => (
        <ArkTabs.Content key={tab.value} value={tab.value}>
          {tab.content}
        </ArkTabs.Content>
      ))}
    </ArkTabs.Root>
  );
}
