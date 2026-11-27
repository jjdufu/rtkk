import { useCallback, useEffect, useState } from "react";
import { AppShell } from "./components/layout/AppShell";
import { ToastProvider } from "./components/primitives/Toast";
import {
  fetchAuthMe,
  fetchSystemOverview,
  logoutSession,
  type AuthMeResponse,
} from "./lib/api/auth";
import { setUnauthorizedHandler } from "./lib/api/client";
import { ThemeProvider } from "./lib/theme";
import { resolveRouteByPath } from "./modules/registry";
import { ArtifactsPage } from "./pages/ArtifactsPage";
import { HarborPage } from "./pages/HarborPage";
import { InventoryPage } from "./pages/InventoryPage";
import { LoginPage } from "./pages/LoginPage";
import { OperationsPage } from "./pages/OperationsPage";
import { OverviewPage } from "./pages/OverviewPage";
import { PkiPage } from "./pages/PkiPage";
import { TemplatesPage } from "./pages/TemplatesPage";

function RouterRoot() {
  const [pathname, setPathname] = useState<string>(() =>
    typeof window !== "undefined" ? window.location.pathname : "/",
  );
  const [checkingAuth, setCheckingAuth] = useState(true);
  const [authInfo, setAuthInfo] = useState<AuthMeResponse | null>(null);
  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);

  const navigate = useCallback((nextPath: string, replace = false) => {
    if (typeof window !== "undefined" && window.location.pathname !== nextPath) {
      if (replace) {
        window.history.replaceState(null, "", nextPath);
      } else {
        window.history.pushState(null, "", nextPath);
      }
    }
    setPathname(nextPath);
  }, []);

  useEffect(() => {
    const onPopState = () => {
      setPathname(window.location.pathname);
    };
    window.addEventListener("popstate", onPopState);
    return () => {
      window.removeEventListener("popstate", onPopState);
    };
  }, []);

  useEffect(() => {
    setUnauthorizedHandler(() => {
      setAuthInfo(null);
      navigate("/login", true);
    });
    return () => {
      setUnauthorizedHandler(null);
    };
  }, [navigate]);

  const refreshSession = useCallback(async () => {
    setCheckingAuth(true);
    try {
      const me = await fetchAuthMe();
      if (me.authenticated) {
        setAuthInfo(me);
        if (window.location.pathname === "/login") {
          navigate("/", true);
        }
        fetchSystemOverview()
          .then((ov) => setActiveJobId(ov.active_mutating_job_id))
          .catch(() => setActiveJobId(null));
      } else {
        setAuthInfo(null);
        navigate("/login", true);
      }
    } catch {
      setAuthInfo(null);
      navigate("/login", true);
    } finally {
      setCheckingAuth(false);
    }
  }, [navigate]);

  useEffect(() => {
    void refreshSession();
  }, [refreshSession]);

  useEffect(() => {
    if (!authInfo) {
      return;
    }
    fetchSystemOverview()
      .then((ov) => setActiveJobId(ov.active_mutating_job_id))
      .catch(() => setActiveJobId(null));
  }, [pathname, authInfo]);

  const handleLoginSuccess = useCallback(() => {
    void refreshSession().then(() => {
      navigate("/", true);
    });
  }, [refreshSession, navigate]);

  const handleLogout = useCallback(async () => {
    try {
      await logoutSession();
    } catch {
      /* session cleared locally regardless */
    }
    setAuthInfo(null);
    setActiveJobId(null);
    navigate("/login", true);
  }, [navigate]);

  if (checkingAuth) {
    return (
      <div
        className="min-h-screen w-full flex items-center justify-center p-6"
        style={{ backgroundColor: "var(--bg)", color: "var(--text-muted)" }}
      >
        <div className="w-full max-w-sm space-y-3" role="status" aria-label="会话检查中">
          <div className="park-skeleton h-10" />
          <div className="park-skeleton h-28" />
        </div>
      </div>
    );
  }

  const route = resolveRouteByPath(pathname);

  if (!authInfo || route.key === "login") {
    return <LoginPage onLoginSuccess={handleLoginSuccess} />;
  }

  return (
    <AppShell
      currentPath={route.path}
      authInfo={authInfo}
      activeJobId={activeJobId}
      onNavigate={(p) => navigate(p)}
      onLogout={() => void handleLogout()}
    >
      {route.key === "overview" ? (
        <OverviewPage
          onNavigate={(p) => navigate(p)}
          onSelectJob={(jobId) => setSelectedJobId(jobId)}
        />
      ) : route.key === "inventory" ? (
        <InventoryPage />
      ) : route.key === "templates" ? (
        <TemplatesPage onNavigate={(p) => navigate(p)} />
      ) : route.key === "artifacts" ? (
        <ArtifactsPage />
      ) : route.key === "harbor" ? (
        <HarborPage onNavigate={(p) => navigate(p)} />
      ) : route.key === "operations" ? (
        <OperationsPage
          selectedJobId={selectedJobId ?? activeJobId}
          onSelectJob={setSelectedJobId}
        />
      ) : (
        <PkiPage />
      )}
    </AppShell>
  );
}

export function App() {
  return (
    <ThemeProvider>
      <ToastProvider>
        <RouterRoot />
      </ToastProvider>
    </ThemeProvider>
  );
}
