import { Menu, Palette, Search } from "lucide-react";
import { Button } from "../primitives/Button";

export interface TopBarProps {
  pageTitle: string;
  activeJobId: string | null;
  onOpenMobileMenu: () => void;
  onOpenThemeEditor: () => void;
  onOpenCommandPalette: () => void;
  onNavigateToOperations: () => void;
}

export function TopBar({
  pageTitle,
  activeJobId,
  onOpenMobileMenu,
  onOpenThemeEditor,
  onOpenCommandPalette,
  onNavigateToOperations,
}: TopBarProps) {
  return (
    <header
      className="h-14 px-4 md:px-6 border-b flex items-center justify-between gap-2 shrink-0"
      style={{
        backgroundColor: "var(--bg)",
        borderColor: "var(--border)",
      }}
    >
      <div className="flex items-center gap-2.5 min-w-0">
        <Button
          variant="secondary"
          size="sm"
          aria-label="导航菜单"
          icon={<Menu size={15} />}
          onClick={onOpenMobileMenu}
          className="md:hidden"
        />
        <h1 className="text-base font-semibold truncate m-0" style={{ color: "var(--text)" }}>
          {pageTitle}
        </h1>
        {activeJobId ? (
          <button
            type="button"
            onClick={onNavigateToOperations}
            className="park-badge tabular-nums font-mono-code cursor-pointer truncate max-w-[160px]"
            title={activeJobId}
          >
            <span
              className="w-2 h-2 inline-block shrink-0"
              style={{ backgroundColor: "var(--warning)", borderRadius: "99px" }}
            />
            <span className="truncate">运行中 · {activeJobId.slice(0, 12)}</span>
          </button>
        ) : null}
      </div>

      <div className="flex items-center gap-2 shrink-0">
        <Button
          variant="secondary"
          size="sm"
          icon={<Search size={13} />}
          onClick={onOpenCommandPalette}
          aria-label="快捷导航"
        >
          <span className="hidden sm:inline">导航</span>
          <span className="font-mono-code text-[11px] opacity-75">⌘K</span>
        </Button>
        <Button
          variant="secondary"
          size="sm"
          icon={<Palette size={14} />}
          onClick={onOpenThemeEditor}
        >
          主题
        </Button>
      </div>
    </header>
  );
}
