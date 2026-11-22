import { useEffect, useState, type ReactNode } from "react";
import { Button } from "./Button";
import { Dialog } from "./Dialog";

export interface ConfirmModalProps {
  open: boolean;
  onOpenChange: (nextOpen: boolean) => void;
  title: string;
  consequence: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  requiredPhrase?: string;
  danger?: boolean;
  busy?: boolean;
  onConfirm: () => void;
}

export function ConfirmModal({
  open,
  onOpenChange,
  title,
  consequence,
  confirmLabel,
  cancelLabel = "取消",
  requiredPhrase,
  danger = true,
  busy = false,
  onConfirm,
}: ConfirmModalProps) {
  const [phraseInput, setPhraseInput] = useState("");

  useEffect(() => {
    if (!open) {
      setPhraseInput("");
    }
  }, [open]);

  const canConfirm = requiredPhrase ? phraseInput.trim() === requiredPhrase : true;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={() => onOpenChange(false)}>
            {cancelLabel}
          </Button>
          <Button
            variant={danger ? "danger" : "primary"}
            disabled={busy || !canConfirm}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="leading-relaxed" style={{ color: "var(--text)" }}>
          {consequence}
        </div>
        {requiredPhrase ? (
          <div>
            <label className="block mb-1.5 text-xs font-medium" style={{ color: "var(--text)" }}>
              确认口令（{requiredPhrase}）
            </label>
            <input
              type="text"
              value={phraseInput}
              onChange={(e) => setPhraseInput(e.target.value)}
              placeholder={requiredPhrase}
              autoComplete="off"
              className="park-input font-mono-code"
            />
          </div>
        ) : null}
      </div>
    </Dialog>
  );
}
