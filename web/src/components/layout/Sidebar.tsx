import {
  FileCode2,
  LayoutDashboard,
  LogOut,
  Package,
  Play,
  Server,
  ShieldCheck,
} from "lucide-react";
import type { ReactNode } from "react";
import type { AuthMeResponse } from "../../lib/api/auth";
import { SIDEBAR_ROUTES, type RouteKey } from "../../modules/registry";
import { Button } from "../primitives/Button";

const ROUTE_ICONS: Record<Exclude<RouteKey, "login">, ReactNode> = {
  overview: <LayoutDashboard size={16} />,
  inventory: <Server size={16} />,
  templates: <FileCode2 size={16} />,
  artifacts: <Package size={16} />,
  operations: <Play size={16} />,
  pki: <ShieldCheck size={16} />,
};

export interface SidebarNavProps {
  activePath: string;
  authInfo: AuthMeResponse | null;
  onNavigate: (path: string) => void;
  onLogout: () => void;
}

export function SidebarNav({ activePath, authInfo, onNavigate, onLogout }: SidebarNavProps) {
  return (
    <div className="h-full flex flex-col justify-between">
      <div>
        <div
          className="px-4 h-14 border-b flex items-center justify-between gap-2"
          style={{ borderColor: "var(--border)" }}
        >
          <div className="flex items-center gap-2.5 min-w-0">
            <span
              className="w-7 h-7 inline-flex items-center justify-center text-xs font-bold shrink-0"
              style={{
                backgroundColor: "var(--accent)",
                color: "var(--accent-fg)",
                borderRadius: "var(--radius-md)",
              }}
            >
              R
            </span>
            <span className="font-semibold text-sm tracking-tight truncate" style={{ color: "var(--text)" }}>
              RTKK
            </span>
          </div>
          {authInfo ? (
            <span
              className="tabular-nums font-mono-code text-[11px] px-1.5 py-0.5 border"
              style={{
                backgroundColor: "var(--surface-2)",
                borderColor: "var(--border)",
                color: "var(--text)",
                borderRadius: "var(--radius-md)",
              }}
            >
              v{authInfo.version}
            </span>
          ) : null}
        </div>

        <nav aria-label="主导航" className="p-2.5 space-y-1">
          {SIDEBAR_ROUTES.map((route) => {
            const active =
              route.path === "/" ? activePath === "/" : activePath.startsWith(route.path);
            const iconNode =
              route.key !== "login" ? ROUTE_ICONS[route.key] : null;
            return (
              <button
                key={route.key}
                type="button"
                aria-current={active ? "page" : undefined}
                data-nav-item={route.key}
                onClick={() => onNavigate(route.path)}
                className="w-full px-3 py-2 flex items-center gap-2.5 text-[13px] font-medium text-left cursor-pointer transition-colors"
                style={{
                  borderRadius: "var(--radius-md)",
                  backgroundColor: active ? "var(--accent)" : "transparent",
                  color: active ? "var(--accent-fg)" : "var(--text)",
                }}
              >
                <span className="shrink-0 inline-flex">{iconNode}</span>
                <span className="truncate">{route.label}</span>
              </button>
            );
          })}
        </nav>
      </div>

      <div className="p-3 border-t space-y-2.5" style={{ borderColor: "var(--border)" }}>
        {authInfo ? (
          <div className="px-1 space-y-0.5 text-[11px]" style={{ color: "var(--text-muted)" }}>
            <div className="tabular-nums font-mono-code truncate" title={authInfo.bind_addr}>
              {authInfo.bind_addr}
            </div>
            <div className="font-mono-code truncate" title={authInfo.work_dir}>
              {authInfo.work_dir}
            </div>
          </div>
        ) : null}
        <Button
          variant="secondary"
          size="sm"
          icon={<LogOut size={13} />}
          onClick={onLogout}
          className="w-full"
        >
          退出
        </Button>
      </div>
    </div>
  );
}

export function DesktopSidebar(props: SidebarNavProps) {
  return (
    <aside
      className="hidden md:flex md:w-56 md:shrink-0 md:flex-col border-r"
      style={{
        backgroundColor: "var(--surface)",
        borderColor: "var(--border)",
      }}
    >
      <SidebarNav {...props} />
    </aside>
  );
}
