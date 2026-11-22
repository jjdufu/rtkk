import { Moon, RotateCcw, Sun } from "lucide-react";
import {
  ACCENT_PRESETS,
  NEUTRAL_PALETTES,
  RADIUS_STEPS,
  useTheme,
  type AccentPreset,
  type NeutralPalette,
} from "../../lib/theme";
import { Button } from "../primitives/Button";
import { Drawer } from "../primitives/Drawer";

export interface ThemeEditorDrawerProps {
  open: boolean;
  onOpenChange: (nextOpen: boolean) => void;
}

const NEUTRAL_KEYS: readonly NeutralPalette[] = ["neutral", "slate", "zinc", "sand"];
const ACCENT_KEYS: readonly AccentPreset[] = ["monochrome", "blue", "teal", "emerald", "amber", "rose"];

export function ThemeEditorDrawer({ open, onOpenChange }: ThemeEditorDrawerProps) {
  const {
    theme,
    accentHex,
    accentFgHex,
    contrast,
    setMode,
    setNeutral,
    setRadius,
    setAccent,
    resetTheme,
  } = useTheme();

  return (
    <Drawer
      open={open}
      onOpenChange={onOpenChange}
      title="主题设置"
      side="right"
      widthClass="max-w-sm"
      footer={
        <Button variant="secondary" size="sm" icon={<RotateCcw size={13} />} onClick={resetTheme}>
          重置默认
        </Button>
      }
    >
      <div className="space-y-5 text-[13px]">
        <section>
          <div className="text-xs font-semibold mb-2" style={{ color: "var(--text)" }}>
            外观模式
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Button
              variant={theme.mode === "light" ? "primary" : "secondary"}
              size="sm"
              icon={<Sun size={14} />}
              onClick={() => setMode("light")}
            >
              浅色
            </Button>
            <Button
              variant={theme.mode === "dark" ? "primary" : "secondary"}
              size="sm"
              icon={<Moon size={14} />}
              onClick={() => setMode("dark")}
            >
              深色
            </Button>
          </div>
        </section>

        <section>
          <div className="text-xs font-semibold mb-2" style={{ color: "var(--text)" }}>
            中性灰阶
          </div>
          <div className="grid grid-cols-2 gap-2">
            {NEUTRAL_KEYS.map((key) => {
              const entry = NEUTRAL_PALETTES[key];
              const active = theme.neutral === key;
              return (
                <Button
                  key={key}
                  variant={active ? "primary" : "secondary"}
                  size="sm"
                  onClick={() => setNeutral(key)}
                >
                  {entry.label}
                </Button>
              );
            })}
          </div>
        </section>

        <section>
          <div className="text-xs font-semibold mb-2" style={{ color: "var(--text)" }}>
            强调色
          </div>
          <div className="grid grid-cols-2 gap-2">
            {ACCENT_KEYS.map((key) => {
              const entry = ACCENT_PRESETS[key];
              const active = theme.accent === key;
              return (
                <Button
                  key={key}
                  variant={active ? "primary" : "secondary"}
                  size="sm"
                  onClick={() => setAccent(key)}
                >
                  <span
                    className="w-3 h-3 inline-block border"
                    style={{
                      backgroundColor: theme.mode === "dark" ? entry.dark : entry.light,
                      borderColor: "var(--border)",
                      borderRadius: "99px",
                    }}
                  />
                  <span>{entry.label}</span>
                </Button>
              );
            })}
          </div>
        </section>

        <section>
          <div className="text-xs font-semibold mb-2" style={{ color: "var(--text)" }}>
            圆角阶梯
          </div>
          <div className="grid grid-cols-5 gap-1.5">
            {RADIUS_STEPS.map((step) => {
              const active = theme.radius === step;
              return (
                <Button
                  key={step}
                  variant={active ? "primary" : "secondary"}
                  size="sm"
                  onClick={() => setRadius(step)}
                  className="tabular-nums px-1.5"
                >
                  {step}
                </Button>
              );
            })}
          </div>
        </section>

        <section className="park-card p-3 space-y-1.5">
          <div className="flex items-center justify-between text-xs">
            <span style={{ color: "var(--text-muted)" }}>对比度（WCAG）</span>
            <span className="tabular-nums font-mono-code font-semibold" style={{ color: "var(--text)" }}>
              {contrast.toFixed(2)}:1
            </span>
          </div>
          <div className="flex items-center justify-between text-xs font-mono-code tabular-nums">
            <span style={{ color: "var(--text-muted)" }}>前景 / 强调色</span>
            <span style={{ color: "var(--text)" }}>
              {accentFgHex} / {accentHex}
            </span>
          </div>
        </section>
      </div>
    </Drawer>
  );
}
