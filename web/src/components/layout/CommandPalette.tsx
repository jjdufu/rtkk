import { ArrowRight } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTheme } from "../../lib/theme";
import { SIDEBAR_ROUTES } from "../../modules/registry";
import { Dialog } from "../primitives/Dialog";

export interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (nextOpen: boolean) => void;
  onNavigate: (path: string) => void;
  onOpenThemeEditor: () => void;
}

interface PaletteAction {
  id: string;
  label: string;
  hint: string;
  run: () => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  onNavigate,
  onOpenThemeEditor,
}: CommandPaletteProps) {
  const { theme, toggleMode } = useTheme();
  const [query, setQuery] = useState("");

  useEffect(() => {
    const onKeyDown = (evt: KeyboardEvent) => {
      if ((evt.metaKey || evt.ctrlKey) && evt.key.toLowerCase() === "k") {
        evt.preventDefault();
        onOpenChange(!open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onOpenChange]);

  useEffect(() => {
    if (!open) {
      setQuery("");
    }
  }, [open]);

  const actions = useMemo<PaletteAction[]>(() => {
    const navItems: PaletteAction[] = SIDEBAR_ROUTES.map((route) => ({
      id: `nav:${route.key}`,
      label: route.label,
      hint: route.path,
      run: () => {
        onOpenChange(false);
        onNavigate(route.path);
      },
    }));
    navItems.push({
      id: "theme:toggle",
      label: theme.mode === "dark" ? "浅色模式" : "深色模式",
      hint: "外观",
      run: () => {
        toggleMode();
        onOpenChange(false);
      },
    });
    navItems.push({
      id: "theme:drawer",
      label: "主题设置",
      hint: "调色板",
      run: () => {
        onOpenChange(false);
        onOpenThemeEditor();
      },
    });
    const q = query.trim().toLowerCase();
    if (!q) {
      return navItems;
    }
    return navItems.filter(
      (item) => item.label.toLowerCase().includes(q) || item.hint.toLowerCase().includes(q),
    );
  }, [query, theme.mode, toggleMode, onNavigate, onOpenChange, onOpenThemeEditor]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange} title="快捷导航" maxWidthClass="max-w-md">
      <div className="space-y-3">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="概览 / 集群配置 / 离线资源"
          autoFocus
          className="park-input"
        />
        <div className="max-h-64 overflow-y-auto space-y-1">
          {actions.length === 0 ? (
            <div className="py-4 text-center text-xs" style={{ color: "var(--text-muted)" }}>
              无匹配项
            </div>
          ) : (
            actions.map((action) => (
              <button
                key={action.id}
                type="button"
                onClick={action.run}
                className="w-full px-3 py-2 flex items-center justify-between gap-2 text-left text-[13px] border cursor-pointer"
                style={{
                  backgroundColor: "var(--surface)",
                  borderColor: "var(--border)",
                  color: "var(--text)",
                  borderRadius: "var(--radius-md)",
                }}
              >
                <span className="font-medium">{action.label}</span>
                <span
                  className="inline-flex items-center gap-1 text-xs font-mono-code"
                  style={{ color: "var(--text-muted)" }}
                >
                  <span>{action.hint}</span>
                  <ArrowRight size={12} />
                </span>
              </button>
            ))
          )}
        </div>
      </div>
    </Dialog>
  );
}
