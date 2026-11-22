import { AlertCircle, Eye, EyeOff, KeyRound, Palette } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { ThemeEditorDrawer } from "../components/layout/ThemeEditorDrawer";
import { Button } from "../components/primitives/Button";
import { fetchHealth, loginWithToken, type HealthResponse } from "../lib/api/auth";
import { ApiClientError } from "../lib/api/client";
import { formatUptimeSecs } from "../lib/format";

export interface LoginPageProps {
  onLoginSuccess: () => void;
}

export function LoginPage({ onLoginSuccess }: LoginPageProps) {
  const [token, setToken] = useState("");
  const [reveal, setReveal] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [themeOpen, setThemeOpen] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    fetchHealth(controller.signal)
      .then((res) => setHealth(res))
      .catch(() => setHealth(null));
    return () => controller.abort();
  }, []);

  const handleSubmit = async (evt: FormEvent<HTMLFormElement>) => {
    evt.preventDefault();
    const trimmed = token.trim();
    if (!trimmed) {
      setErrorMsg("管理令牌不能为空");
      return;
    }
    setSubmitting(true);
    setErrorMsg(null);
    try {
      const res = await loginWithToken(trimmed);
      if (res.authenticated) {
        onLoginSuccess();
      } else {
        setErrorMsg("认证未通过");
      }
    } catch (err) {
      if (err instanceof ApiClientError) {
        setErrorMsg(`${err.message} (${err.code})`);
      } else {
        setErrorMsg("网络请求异常");
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      className="min-h-screen w-full flex flex-col justify-between p-4"
      style={{ backgroundColor: "var(--bg)", color: "var(--text)" }}
    >
      <div className="flex items-center justify-between max-w-5xl w-full mx-auto py-2">
        <div className="flex items-center gap-2">
          <span
            className="w-7 h-7 inline-flex items-center justify-center text-xs font-bold"
            style={{
              backgroundColor: "var(--accent)",
              color: "var(--accent-fg)",
              borderRadius: "var(--radius-md)",
            }}
          >
            R
          </span>
          <span className="font-semibold text-sm">RTKK</span>
          {health ? (
            <span className="park-badge tabular-nums font-mono-code">
              v{health.version} · {formatUptimeSecs(health.uptime_secs)}
            </span>
          ) : null}
        </div>
        <Button
          variant="secondary"
          size="sm"
          icon={<Palette size={14} />}
          onClick={() => setThemeOpen(true)}
        >
          主题
        </Button>
      </div>

      <div className="w-full max-w-md mx-auto my-8">
        <div className="park-card p-6 space-y-5">
          <div className="flex items-center gap-2.5">
            <span
              className="w-9 h-9 inline-flex items-center justify-center border shrink-0"
              style={{
                backgroundColor: "var(--bg)",
                borderColor: "var(--border)",
                borderRadius: "var(--radius-md)",
              }}
            >
              <KeyRound size={17} />
            </span>
            <div>
              <h1 className="text-base font-semibold m-0" style={{ color: "var(--text)" }}>
                控制台认证
              </h1>
              <p className="text-xs m-0 mt-0.5 font-mono-code" style={{ color: "var(--text-muted)" }}>
                .rtkk-daemon/admin.token
              </p>
            </div>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label
                htmlFor="rtkk-admin-token"
                className="block mb-1.5 text-xs font-medium"
                style={{ color: "var(--text)" }}
              >
                管理令牌
              </label>
              <div className="relative flex items-center">
                <input
                  id="rtkk-admin-token"
                  name="token"
                  type={reveal ? "text" : "password"}
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="粘贴 43 位管理令牌"
                  autoComplete="current-password"
                  autoFocus
                  className="park-input font-mono-code pr-9"
                />
                <button
                  type="button"
                  aria-label={reveal ? "隐藏令牌" : "显示令牌"}
                  onClick={() => setReveal((prev) => !prev)}
                  className="absolute right-2 p-1 inline-flex items-center cursor-pointer"
                  style={{ color: "var(--text-muted)" }}
                >
                  {reveal ? <EyeOff size={15} /> : <Eye size={15} />}
                </button>
              </div>
            </div>

            {errorMsg ? (
              <div
                role="alert"
                className="p-2.5 border flex items-start gap-2 text-xs"
                style={{
                  backgroundColor: "var(--bg)",
                  borderColor: "var(--danger)",
                  color: "var(--danger)",
                  borderRadius: "var(--radius-md)",
                }}
              >
                <AlertCircle size={15} className="shrink-0 mt-0.5" />
                <span className="break-words">{errorMsg}</span>
              </div>
            ) : null}

            <Button
              type="submit"
              variant="primary"
              disabled={submitting}
              className="w-full"
            >
              {submitting ? "验证中…" : "登录"}
            </Button>
          </form>
        </div>
      </div>

      <div className="text-center text-xs py-2 tabular-nums font-mono-code" style={{ color: "var(--text-muted)" }}>
        127.0.0.1:19912
      </div>

      <ThemeEditorDrawer open={themeOpen} onOpenChange={setThemeOpen} />
    </div>
  );
}
