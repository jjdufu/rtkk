import { AlertTriangle, Inbox } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "./Button";

export interface StateViewProps {
  loading: boolean;
  error: string | null;
  empty?: boolean;
  emptyTitle?: string;
  emptyActionLabel?: string;
  onEmptyAction?: () => void;
  onRetry?: () => void;
  skeletonRows?: number;
  children: ReactNode;
}

export function StateView({
  loading,
  error,
  empty = false,
  emptyTitle = "暂无记录",
  emptyActionLabel,
  onEmptyAction,
  onRetry,
  skeletonRows = 4,
  children,
}: StateViewProps) {
  if (loading) {
    return (
      <div className="space-y-3" role="status" aria-label="加载中">
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <div className="park-skeleton h-20" />
          <div className="park-skeleton h-20" />
          <div className="park-skeleton h-20" />
        </div>
        {Array.from({ length: skeletonRows }).map((_, idx) => (
          <div key={idx} className="park-skeleton h-12" />
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <div className="park-card p-6 flex flex-col items-center justify-center text-center gap-3">
        <AlertTriangle size={24} style={{ color: "var(--danger)" }} />
        <div className="text-sm font-medium" style={{ color: "var(--text)" }}>
          加载失败
        </div>
        <div className="text-xs max-w-md break-words" style={{ color: "var(--text-muted)" }}>
          {error}
        </div>
        {onRetry ? (
          <Button variant="primary" size="sm" onClick={onRetry}>
            重试
          </Button>
        ) : null}
      </div>
    );
  }

  if (empty) {
    return (
      <div className="park-card p-8 flex flex-col items-center justify-center text-center gap-3">
        <Inbox size={24} style={{ color: "var(--text-muted)" }} />
        <div className="text-sm font-medium" style={{ color: "var(--text-muted)" }}>
          {emptyTitle}
        </div>
        {emptyActionLabel && onEmptyAction ? (
          <Button variant="primary" size="sm" onClick={onEmptyAction}>
            {emptyActionLabel}
          </Button>
        ) : null}
      </div>
    );
  }

  return <>{children}</>;
}
