import { Dialog as ArkDialog, Portal } from "@ark-ui/react";
import { X } from "lucide-react";
import type { ReactNode } from "react";

export interface DialogProps {
  open: boolean;
  onOpenChange: (nextOpen: boolean) => void;
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
  maxWidthClass?: string;
}

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  maxWidthClass = "max-w-lg",
}: DialogProps) {
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
          style={{ backgroundColor: "rgba(9, 9, 11, 0.55)", zIndex: 80 }}
        />
        <ArkDialog.Positioner
          className="fixed inset-0 flex items-center justify-center p-4 overflow-y-auto"
          style={{ zIndex: 81 }}
        >
          <ArkDialog.Content
            className={`w-full ${maxWidthClass} border shadow-xl p-5`}
            style={{
              backgroundColor: "var(--bg)",
              color: "var(--text)",
              borderColor: "var(--border)",
              borderRadius: "var(--radius-md)",
            }}
          >
            <div className="flex items-start justify-between gap-3 mb-3">
              <div className="min-w-0">
                <ArkDialog.Title
                  className="text-base font-semibold leading-snug"
                  style={{ color: "var(--text)" }}
                >
                  {title}
                </ArkDialog.Title>
                {description ? (
                  <ArkDialog.Description
                    className="mt-1 text-xs leading-relaxed"
                    style={{ color: "var(--text-muted)" }}
                  >
                    {description}
                  </ArkDialog.Description>
                ) : null}
              </div>
              <ArkDialog.CloseTrigger
                aria-label="关闭"
                className="inline-flex items-center justify-center p-1 cursor-pointer"
                style={{ color: "var(--text-muted)", borderRadius: "var(--radius-md)" }}
              >
                <X size={16} />
              </ArkDialog.CloseTrigger>
            </div>

            <div className="text-[13px]">{children}</div>

            {footer ? <div className="mt-5 flex items-center justify-end gap-2">{footer}</div> : null}
          </ArkDialog.Content>
        </ArkDialog.Positioner>
      </Portal>
    </ArkDialog.Root>
  );
}
