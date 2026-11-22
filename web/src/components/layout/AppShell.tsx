import { useState, type ReactNode } from "react";
import type { AuthMeResponse } from "../../lib/api/auth";
import { resolveRouteByPath } from "../../modules/registry";
import { Drawer } from "../primitives/Drawer";
import { CommandPalette } from "./CommandPalette";
import { DesktopSidebar, SidebarNav } from "./Sidebar";
import { ThemeEditorDrawer } from "./ThemeEditorDrawer";
import { TopBar } from "./TopBar";

export interface AppShellProps {
  currentPath: string;
  authInfo: AuthMeResponse | null;
  activeJobId: string | null;
  onNavigate: (path: string) => void;
  onLogout: () => void;
  children: ReactNode;
}

export function AppShell({
  currentPath,
  authInfo,
  activeJobId,
  onNavigate,
  onLogout,
  children,
}: AppShellProps) {
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [themeOpen, setThemeOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);

  const activeRoute = resolveRouteByPath(currentPath);

  const handleNavigate = (path: string) => {
    setMobileMenuOpen(false);
    onNavigate(path);
  };

  return (
    <div className="min-h-screen w-full flex overflow-x-hidden" style={{ backgroundColor: "var(--bg)" }}>
      <DesktopSidebar
        activePath={currentPath}
        authInfo={authInfo}
        onNavigate={handleNavigate}
        onLogout={onLogout}
      />

      <Drawer
        open={mobileMenuOpen}
        onOpenChange={setMobileMenuOpen}
        title="导航"
        side="left"
        widthClass="max-w-[240px]"
      >
        <div className="-m-4 h-full">
          <SidebarNav
            activePath={currentPath}
            authInfo={authInfo}
            onNavigate={handleNavigate}
            onLogout={onLogout}
          />
        </div>
      </Drawer>

      <div className="flex-1 min-w-0 flex flex-col">
        <TopBar
          pageTitle={activeRoute.label}
          activeJobId={activeJobId}
          onOpenMobileMenu={() => setMobileMenuOpen(true)}
          onOpenThemeEditor={() => setThemeOpen(true)}
          onOpenCommandPalette={() => setPaletteOpen(true)}
          onNavigateToOperations={() => handleNavigate("/operations")}
        />
        <main className="flex-1 min-w-0 p-4 md:p-6 overflow-x-hidden">{children}</main>
      </div>

      <ThemeEditorDrawer open={themeOpen} onOpenChange={setThemeOpen} />
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        onNavigate={handleNavigate}
        onOpenThemeEditor={() => setThemeOpen(true)}
      />
    </div>
  );
}
