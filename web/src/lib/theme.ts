import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { safeGetStorageItem, safeSetStorageItem } from "./storage";

export type ThemeMode = "light" | "dark";
export type NeutralPalette = "neutral" | "slate" | "zinc" | "sand";
export type RadiusStep = "0px" | "4px" | "6px" | "10px" | "99px";
export type AccentPreset = "monochrome" | "blue" | "teal" | "emerald" | "amber" | "rose";

export interface ThemePreferences {
  mode: ThemeMode;
  neutral: NeutralPalette;
  radius: RadiusStep;
  accent: AccentPreset;
}

export const THEME_STORAGE_KEY = "rtkk.theme.v1";

export const DEFAULT_THEME: ThemePreferences = {
  mode: "light",
  neutral: "zinc",
  radius: "6px",
  accent: "monochrome",
};

interface PaletteVars {
  bg: string;
  surface: string;
  surface2: string;
  border: string;
  text: string;
  textMuted: string;
}

export const NEUTRAL_PALETTES: Record<NeutralPalette, { label: string; light: PaletteVars; dark: PaletteVars }> = {
  neutral: {
    label: "Neutral",
    light: {
      bg: "#fcfcfc",
      surface: "#f5f5f5",
      surface2: "#e5e5e5",
      border: "#d4d4d4",
      text: "#171717",
      textMuted: "#525252",
    },
    dark: {
      bg: "#0a0a0a",
      surface: "#141414",
      surface2: "#1f1f1f",
      border: "#2e2e2e",
      text: "#fafafa",
      textMuted: "#a3a3a3",
    },
  },
  slate: {
    label: "Slate",
    light: {
      bg: "#f8fafc",
      surface: "#f1f5f9",
      surface2: "#e2e8f0",
      border: "#cbd5e1",
      text: "#0f172a",
      textMuted: "#475569",
    },
    dark: {
      bg: "#090d16",
      surface: "#0f172a",
      surface2: "#1e293b",
      border: "#334155",
      text: "#f8fafc",
      textMuted: "#94a3b8",
    },
  },
  zinc: {
    label: "Zinc",
    light: {
      bg: "#fcfcfc",
      surface: "#f4f4f5",
      surface2: "#e4e4e7",
      border: "#d4d4d8",
      text: "#18181b",
      textMuted: "#52525b",
    },
    dark: {
      bg: "#09090b",
      surface: "#121215",
      surface2: "#18181b",
      border: "#27272a",
      text: "#fafafa",
      textMuted: "#a1a1aa",
    },
  },
  sand: {
    label: "Sand",
    light: {
      bg: "#fdfdfc",
      surface: "#f5f5f0",
      surface2: "#e7e5df",
      border: "#d6d3cb",
      text: "#1c1917",
      textMuted: "#57534e",
    },
    dark: {
      bg: "#0c0a09",
      surface: "#171412",
      surface2: "#211d1a",
      border: "#332d29",
      text: "#fafaf9",
      textMuted: "#a8a29e",
    },
  },
};

export const ACCENT_PRESETS: Record<AccentPreset, { label: string; light: string; dark: string; swatch: string }> = {
  monochrome: {
    label: "Monochrome",
    light: "#18181b",
    dark: "#fafafa",
    swatch: "#18181b",
  },
  blue: {
    label: "Blue",
    light: "#1d4ed8",
    dark: "#60a5fa",
    swatch: "#2563eb",
  },
  teal: {
    label: "Teal",
    light: "#0f766e",
    dark: "#2dd4bf",
    swatch: "#0d9488",
  },
  emerald: {
    label: "Emerald",
    light: "#047857",
    dark: "#34d399",
    swatch: "#059669",
  },
  amber: {
    label: "Amber",
    light: "#b45309",
    dark: "#fbbf24",
    swatch: "#d97706",
  },
  rose: {
    label: "Rose",
    light: "#be123c",
    dark: "#fb7185",
    swatch: "#e11d48",
  },
};

export const RADIUS_STEPS: readonly RadiusStep[] = ["0px", "4px", "6px", "10px", "99px"];

export function parseHexRgb(hex: string): [number, number, number] | null {
  const cleaned = hex.trim().replace(/^#/, "");
  const normalized =
    cleaned.length === 3
      ? cleaned
          .split("")
          .map((ch) => ch + ch)
          .join("")
      : cleaned;
  if (!/^[0-9a-fA-F]{6}$/.test(normalized)) {
    return null;
  }
  const r = Number.parseInt(normalized.slice(0, 2), 16) / 255;
  const g = Number.parseInt(normalized.slice(2, 4), 16) / 255;
  const b = Number.parseInt(normalized.slice(4, 6), 16) / 255;
  return [r, g, b];
}

export function relativeLuminance(rgb: [number, number, number]): number {
  const linearize = (c: number): number => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const [r, g, b] = rgb;
  return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b);
}

export function wcagContrastRatio(hexA: string, hexB: string): number {
  const rgbA = parseHexRgb(hexA);
  const rgbB = parseHexRgb(hexB);
  if (!rgbA || !rgbB) {
    return 1;
  }
  const lumA = relativeLuminance(rgbA);
  const lumB = relativeLuminance(rgbB);
  const lighter = Math.max(lumA, lumB);
  const darker = Math.min(lumA, lumB);
  return (lighter + 0.05) / (darker + 0.05);
}

export function resolveAccessibleForeground(accentHex: string): string {
  const lightCandidate = "#ffffff";
  const darkCandidate = "#09090b";
  const lightContrast = wcagContrastRatio(accentHex, lightCandidate);
  const darkContrast = wcagContrastRatio(accentHex, darkCandidate);
  return darkContrast >= lightContrast ? darkCandidate : lightCandidate;
}

function isThemeMode(v: unknown): v is ThemeMode {
  return v === "light" || v === "dark";
}

function isNeutralPalette(v: unknown): v is NeutralPalette {
  return v === "neutral" || v === "slate" || v === "zinc" || v === "sand";
}

function isRadiusStep(v: unknown): v is RadiusStep {
  return v === "0px" || v === "4px" || v === "6px" || v === "10px" || v === "99px";
}

function isAccentPreset(v: unknown): v is AccentPreset {
  return (
    v === "monochrome" ||
    v === "blue" ||
    v === "teal" ||
    v === "emerald" ||
    v === "amber" ||
    v === "rose"
  );
}

export function loadStoredTheme(): ThemePreferences {
  const raw = safeGetStorageItem(THEME_STORAGE_KEY);
  if (!raw) {
    return DEFAULT_THEME;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) {
      return DEFAULT_THEME;
    }
    const record = parsed as Record<string, unknown>;
    return {
      mode: isThemeMode(record["mode"]) ? record["mode"] : DEFAULT_THEME.mode,
      neutral: isNeutralPalette(record["neutral"]) ? record["neutral"] : DEFAULT_THEME.neutral,
      radius: isRadiusStep(record["radius"]) ? record["radius"] : DEFAULT_THEME.radius,
      accent: isAccentPreset(record["accent"]) ? record["accent"] : DEFAULT_THEME.accent,
    };
  } catch {
    return DEFAULT_THEME;
  }
}

export function applyThemeToDocument(prefs: ThemePreferences): { accentHex: string; accentFgHex: string; contrast: number } {
  const palette = NEUTRAL_PALETTES[prefs.neutral][prefs.mode];
  const accentEntry = ACCENT_PRESETS[prefs.accent];
  const accentHex = prefs.mode === "dark" ? accentEntry.dark : accentEntry.light;
  const accentFgHex = resolveAccessibleForeground(accentHex);
  const contrast = wcagContrastRatio(accentHex, accentFgHex);

  if (typeof document !== "undefined") {
    const root = document.documentElement;
    if (prefs.mode === "dark") {
      root.setAttribute("data-theme", "dark");
    } else {
      root.removeAttribute("data-theme");
    }
    root.style.setProperty("--bg", palette.bg);
    root.style.setProperty("--surface", palette.surface);
    root.style.setProperty("--surface-2", palette.surface2);
    root.style.setProperty("--border", palette.border);
    root.style.setProperty("--text", palette.text);
    root.style.setProperty("--text-muted", palette.textMuted);
    root.style.setProperty("--accent", accentHex);
    root.style.setProperty("--accent-fg", accentFgHex);
    root.style.setProperty("--radius-md", prefs.radius);
  }

  return { accentHex, accentFgHex, contrast };
}

interface ThemeContextValue {
  theme: ThemePreferences;
  accentHex: string;
  accentFgHex: string;
  contrast: number;
  setMode: (mode: ThemeMode) => void;
  toggleMode: () => void;
  setNeutral: (neutral: NeutralPalette) => void;
  setRadius: (radius: RadiusStep) => void;
  setAccent: (accent: AccentPreset) => void;
  resetTheme: () => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setTheme] = useState<ThemePreferences>(() => loadStoredTheme());

  const resolved = useMemo(() => {
    const palette = NEUTRAL_PALETTES[theme.neutral][theme.mode];
    const accentEntry = ACCENT_PRESETS[theme.accent];
    const accentHex = theme.mode === "dark" ? accentEntry.dark : accentEntry.light;
    const accentFgHex = resolveAccessibleForeground(accentHex);
    const contrast = wcagContrastRatio(accentHex, accentFgHex);
    return { palette, accentHex, accentFgHex, contrast };
  }, [theme]);

  useEffect(() => {
    applyThemeToDocument(theme);
    safeSetStorageItem(THEME_STORAGE_KEY, JSON.stringify(theme));
  }, [theme]);

  const setMode = useCallback((mode: ThemeMode) => {
    setTheme((prev) => ({ ...prev, mode }));
  }, []);

  const toggleMode = useCallback(() => {
    setTheme((prev) => ({ ...prev, mode: prev.mode === "dark" ? "light" : "dark" }));
  }, []);

  const setNeutral = useCallback((neutral: NeutralPalette) => {
    setTheme((prev) => ({ ...prev, neutral }));
  }, []);

  const setRadius = useCallback((radius: RadiusStep) => {
    setTheme((prev) => ({ ...prev, radius }));
  }, []);

  const setAccent = useCallback((accent: AccentPreset) => {
    setTheme((prev) => ({ ...prev, accent }));
  }, []);

  const resetTheme = useCallback(() => {
    setTheme(DEFAULT_THEME);
  }, []);

  const value = useMemo<ThemeContextValue>(
    () => ({
      theme,
      accentHex: resolved.accentHex,
      accentFgHex: resolved.accentFgHex,
      contrast: resolved.contrast,
      setMode,
      toggleMode,
      setNeutral,
      setRadius,
      setAccent,
      resetTheme,
    }),
    [theme, resolved.accentHex, resolved.accentFgHex, resolved.contrast, setMode, toggleMode, setNeutral, setRadius, setAccent, resetTheme],
  );

  return createElement(ThemeContext.Provider, { value }, children);
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error("useTheme must be called inside ThemeProvider");
  }
  return ctx;
}
