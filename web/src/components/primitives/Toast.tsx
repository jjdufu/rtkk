import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react";

export type ToastTone = "info" | "success" | "error";

interface ToastEntry {
  id: number;
  tone: ToastTone;
  text: string;
}

interface ToastContextValue {
  pushToast: (tone: ToastTone, text: string) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastEntry[]>([]);
  const seqRef = useRef(1);

  const dismiss = useCallback((id: number) => {
    setItems((prev) => prev.filter((item) => item.id !== id));
  }, []);

  const pushToast = useCallback(
    (tone: ToastTone, text: string) => {
      const id = seqRef.current;
      seqRef.current += 1;
      setItems((prev) => [...prev.slice(-3), { id, tone, text }]);
      window.setTimeout(() => {
        dismiss(id);
      }, 3600);
    },
    [dismiss],
  );

  const value = useMemo<ToastContextValue>(() => ({ pushToast }), [pushToast]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div
        aria-live="polite"
        className="fixed bottom-4 right-4 flex flex-col gap-2 max-w-sm pointer-events-none"
        style={{ zIndex: 95 }}
      >
        {items.map((item) => (
          <div
            key={item.id}
            role="status"
            className="pointer-events-auto px-3.5 py-2.5 border shadow-lg flex items-center gap-2.5 text-[13px]"
            style={{
              backgroundColor: "var(--bg)",
              color: "var(--text)",
              borderColor:
                item.tone === "error"
                  ? "var(--danger)"
                  : item.tone === "success"
                    ? "var(--success)"
                    : "var(--border)",
              borderRadius: "var(--radius-md)",
            }}
          >
            <span
              className="shrink-0 inline-flex"
              style={{
                color:
                  item.tone === "error"
                    ? "var(--danger)"
                    : item.tone === "success"
                      ? "var(--success)"
                      : "var(--text)",
              }}
            >
              {item.tone === "error" ? (
                <AlertCircle size={15} />
              ) : item.tone === "success" ? (
                <CheckCircle2 size={15} />
              ) : (
                <Info size={15} />
              )}
            </span>
            <span className="flex-1 break-words">{item.text}</span>
            <button
              type="button"
              aria-label="关闭"
              onClick={() => dismiss(item.id)}
              className="shrink-0 p-0.5 cursor-pointer"
              style={{ color: "var(--text-muted)" }}
            >
              <X size={13} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used within ToastProvider");
  }
  return ctx;
}
