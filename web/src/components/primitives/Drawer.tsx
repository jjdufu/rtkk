import { Dialog as ArkDialog, Portal } from "@ark-ui/react";
import { X } from "lucide-react";
import type { ReactNode } from "react";

export interface DrawerProps {
  open: boolean;
  onOpenChange: (nextOpen: boolean) => void;
  title: string;
  side?: "left" | "right";
  widthClass?: string;
  children: ReactNode;
  footer?: ReactNode;
}

export function Drawer({
  open,
  onOpenChange,
  title,
  side = "right",
  widthClass = "max-w-md",
  children,
  footer,
}: DrawerProps) {
  const justifyClass = side === "left" ? "justify-start" : "justify-end";
  const borderSideClass = side === "left" ? "border-r" : "border-l";

  return (
    <ArkDialog.Root
      open={open}
      onOpenChange={(details) => onOpenChange(details.open)}
      lazyMount
      unmountOnExit
    >
      <Portal>
        <ArkDialog.Backdrop
          className="fixed inset-0"
          style={{ backgroundColor: "rgba(9, 9, 11, 0.5)", zIndex: 75 }}
        />
        <ArkDialog.Positioner
          className={`fixed inset-0 flex ${justifyClass}`}
          style={{ zIndex: 76 }}
        >
          <ArkDialog.Content
            className={`h-full w-full ${widthClass} flex flex-col ${borderSideClass} shadow-2xl`}
            style={{
              backgroundColor: "var(--bg)",
              color: "var(--text)",
              borderColor: "var(--border)",
            }}
          >
            <div
              className="px-4 py-3.5 border-b flex items-center justify-between gap-2 shrink-0"
              style={{ borderColor: "var(--border)" }}
            >
              <ArkDialog.Title className="text-sm font-semibold" style={{ color: "var(--text)" }}>
                {title}
              </ArkDialog.Title>
              <ArkDialog.CloseTrigger
                aria-label="关闭"
                className="inline-flex items-center justify-center p-1 cursor-pointer"
                style={{ color: "var(--text-muted)", borderRadius: "var(--radius-md)" }}
              >
                <X size={16} />
              </ArkDialog.CloseTrigger>
            </div>

            <div className="flex-1 overflow-y-auto p-4">{children}</div>

            {footer ? (
              <div
                className="px-4 py-3 border-t flex items-center justify-end gap-2 shrink-0"
                style={{ borderColor: "var(--border)" }}
              >
                {footer}
              </div>
            ) : null}
          </ArkDialog.Content>
        </ArkDialog.Positioner>
      </Portal>
    </ArkDialog.Root>
  );
}
