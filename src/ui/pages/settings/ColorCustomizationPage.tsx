import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Download, Edit3, RotateCcw, Trash2, Upload } from "lucide-react";
import { useTheme } from "../../../core/theme/ThemeContext";
import {
  getCustomColorPresets,
  getCustomColors,
  setCustomColors as persistCustomColors,
  setCustomColorPresets,
} from "../../../core/storage/appState";
import type { CustomColorPreset, CustomColors } from "../../../core/storage/schemas";
import { cn, interactive, radius } from "../../design-tokens";
import { toast } from "../../components/toast";
import { useI18n } from "../../../core/i18n/context";

const COLOR_TOKENS = [
  {
    key: "surface" as const,
    label: "Surface",
    description: "Page backgrounds",
    defaultValue: "#050505",
    group: "backgrounds",
  },
  {
    key: "surfaceEl" as const,
    label: "Surface Elevated",
    description: "Cards, modals, raised elements",
    defaultValue: "#0a0a0a",
    group: "backgrounds",
  },
  {
    key: "nav" as const,
    label: "Navigation",
    description: "Top & bottom bars",
    defaultValue: "#0a0a0a",
    group: "backgrounds",
  },
  {
    key: "fg" as const,
    label: "Foreground",
    description: "Text, borders, overlays",
    defaultValue: "#ffffff",
    group: "content",
  },
  {
    key: "accent" as const,
    label: "Accent",
    description: "Primary actions, success",
    defaultValue: "#34d399",
    group: "semantic",
  },
  {
    key: "info" as const,
    label: "Info",
    description: "Informational states, links",
    defaultValue: "#3b82f6",
    group: "semantic",
  },
  {
    key: "warning" as const,
    label: "Warning",
    description: "Caution states, alerts",
    defaultValue: "#f59e0b",
    group: "semantic",
  },
  {
    key: "danger" as const,
    label: "Danger",
    description: "Destructive actions, errors",
    defaultValue: "#ef4444",
    group: "semantic",
  },
  {
    key: "secondary" as const,
    label: "Secondary",
    description: "AI features, creative tools",
    defaultValue: "#a78bfa",
    group: "semantic",
  },
] as const;

type ColorKey = (typeof COLOR_TOKENS)[number]["key"];

const TOKEN_GROUPS = [
  { id: "backgrounds", label: "Backgrounds" },
  { id: "content", label: "Content" },
  { id: "semantic", label: "Semantic" },
] as const;

const DEFAULTS = Object.fromEntries(COLOR_TOKENS.map((t) => [t.key, t.defaultValue])) as Record<
  ColorKey,
  string
>;

interface Preset {
  name: string;
  colors: Record<ColorKey, string>;
}

const PRESETS: Preset[] = [
  {
    name: "Default Dark",
    colors: {
      surface: "#050505",
      surfaceEl: "#0a0a0a",
      nav: "#0a0a0a",
      fg: "#ffffff",
      accent: "#34d399",
      info: "#3b82f6",
      warning: "#f59e0b",
      danger: "#ef4444",
      secondary: "#a78bfa",
    },
  },
  {
    name: "Midnight Blue",
    colors: {
      surface: "#0a0e1a",
      surfaceEl: "#111827",
      nav: "#0d1120",
      fg: "#e2e8f0",
      accent: "#60a5fa",
      info: "#818cf8",
      warning: "#fbbf24",
      danger: "#f87171",
      secondary: "#c084fc",
    },
  },
  {
    name: "Warm Earth",
    colors: {
      surface: "#1a1410",
      surfaceEl: "#231c15",
      nav: "#1a1410",
      fg: "#f5e6d3",
      accent: "#d4a574",
      info: "#7cb4c4",
      warning: "#e6a23c",
      danger: "#c45c5c",
      secondary: "#c4a0e0",
    },
  },
  {
    name: "Purple Haze",
    colors: {
      surface: "#0d0815",
      surfaceEl: "#150f20",
      nav: "#0d0815",
      fg: "#e8dff5",
      accent: "#a78bfa",
      info: "#67e8f9",
      warning: "#fcd34d",
      danger: "#fb7185",
      secondary: "#c4b5fd",
    },
  },
  {
    name: "Rose Pine",
    colors: {
      surface: "#191724",
      surfaceEl: "#1f1d2e",
      nav: "#191724",
      fg: "#e0def4",
      accent: "#c4a7e7",
      info: "#9ccfd8",
      warning: "#f6c177",
      danger: "#eb6f92",
      secondary: "#c4a7e7",
    },
  },
  {
    name: "Tokyo Night",
    colors: {
      surface: "#1a1b26",
      surfaceEl: "#24283b",
      nav: "#1a1b26",
      fg: "#c0caf5",
      accent: "#7aa2f7",
      info: "#2ac3de",
      warning: "#e0af68",
      danger: "#f7768e",
      secondary: "#bb9af7",
    },
  },
  {
    name: "Catppuccin",
    colors: {
      surface: "#1e1e2e",
      surfaceEl: "#313244",
      nav: "#1e1e2e",
      fg: "#cdd6f4",
      accent: "#a6e3a1",
      info: "#89b4fa",
      warning: "#f9e2af",
      danger: "#f38ba8",
      secondary: "#cba6f7",
    },
  },
  {
    name: "Gruvbox",
    colors: {
      surface: "#1d2021",
      surfaceEl: "#282828",
      nav: "#1d2021",
      fg: "#ebdbb2",
      accent: "#b8bb26",
      info: "#83a598",
      warning: "#fabd2f",
      danger: "#fb4934",
      secondary: "#d3869b",
    },
  },
  {
    name: "Nord",
    colors: {
      surface: "#2e3440",
      surfaceEl: "#3b4252",
      nav: "#2e3440",
      fg: "#eceff4",
      accent: "#a3be8c",
      info: "#88c0d0",
      warning: "#ebcb8b",
      danger: "#bf616a",
      secondary: "#b48ead",
    },
  },
  {
    name: "Dracula",
    colors: {
      surface: "#282a36",
      surfaceEl: "#44475a",
      nav: "#282a36",
      fg: "#f8f8f2",
      accent: "#50fa7b",
      info: "#8be9fd",
      warning: "#f1fa8c",
      danger: "#ff5555",
      secondary: "#bd93f9",
    },
  },
  {
    name: "Solarized",
    colors: {
      surface: "#002b36",
      surfaceEl: "#073642",
      nav: "#002b36",
      fg: "#fdf6e3",
      accent: "#859900",
      info: "#268bd2",
      warning: "#b58900",
      danger: "#dc322f",
      secondary: "#6c71c4",
    },
  },
  {
    name: "Ayu Dark",
    colors: {
      surface: "#0d1017",
      surfaceEl: "#131721",
      nav: "#0d1017",
      fg: "#bfbdb6",
      accent: "#e6b450",
      info: "#59c2ff",
      warning: "#ffb454",
      danger: "#d95757",
      secondary: "#d2a6ff",
    },
  },
  {
    name: "One Dark",
    colors: {
      surface: "#21252b",
      surfaceEl: "#282c34",
      nav: "#21252b",
      fg: "#abb2bf",
      accent: "#98c379",
      info: "#61afef",
      warning: "#e5c07b",
      danger: "#e06c75",
      secondary: "#c678dd",
    },
  },
  {
    name: "Vesper",
    colors: {
      surface: "#101010",
      surfaceEl: "#1c1c1c",
      nav: "#101010",
      fg: "#b0b0b0",
      accent: "#ffc799",
      info: "#8eb8e2",
      warning: "#deb887",
      danger: "#d08770",
      secondary: "#c4a0e0",
    },
  },
  {
    name: "Cyber Neon",
    colors: {
      surface: "#0a0a12",
      surfaceEl: "#12121e",
      nav: "#0a0a12",
      fg: "#e4e4f0",
      accent: "#00ffaa",
      info: "#00d4ff",
      warning: "#ffe600",
      danger: "#ff2e6c",
      secondary: "#bf5af2",
    },
  },
  {
    name: "Monochrome",
    colors: {
      surface: "#111111",
      surfaceEl: "#1a1a1a",
      nav: "#111111",
      fg: "#e0e0e0",
      accent: "#ffffff",
      info: "#a0a0a0",
      warning: "#c8c8c8",
      danger: "#808080",
      secondary: "#b0b0b0",
    },
  },
];

interface PresetExportPayload {
  type: "color-preset";
  version: 1;
  preset: {
    name: string;
    colors: Record<ColorKey, string>;
  };
}

function isValidHex(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}

function cssVar(key: string): string {
  return `--color-${key.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase())}`;
}

function normalizeCustomColors(colors?: CustomColors): CustomColors {
  const next: CustomColors = {};
  for (const token of COLOR_TOKENS) {
    const value = colors?.[token.key];
    if (value && isValidHex(value)) {
      next[token.key] = value;
    }
  }
  return next;
}

function normalizePreset(preset: CustomColorPreset): CustomColorPreset {
  return {
    ...preset,
    name: preset.name.trim(),
    colors: normalizeCustomColors(preset.colors),
  };
}

function presetsEqual(a: CustomColorPreset[], b: CustomColorPreset[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    const left = normalizePreset(a[i]);
    const right = normalizePreset(b[i]);
    if (
      left.id !== right.id ||
      left.name !== right.name ||
      left.createdAt !== right.createdAt ||
      JSON.stringify(left.colors) !== JSON.stringify(right.colors)
    ) {
      return false;
    }
  }
  return true;
}

function colorsEqual(a: CustomColors, b: CustomColors): boolean {
  return JSON.stringify(normalizeCustomColors(a)) === JSON.stringify(normalizeCustomColors(b));
}

function buildPresetColors(colors: CustomColors): Record<ColorKey, string> {
  return Object.fromEntries(
    COLOR_TOKENS.map((token) => [token.key, colors[token.key] ?? DEFAULTS[token.key]]),
  ) as Record<ColorKey, string>;
}

function parseImportedPreset(value: unknown): { name: string; colors: CustomColors } | null {
  if (!value || typeof value !== "object") return null;

  const data = value as Record<string, unknown>;
  const preset =
    data.preset && typeof data.preset === "object"
      ? (data.preset as Record<string, unknown>)
      : data;

  const name = typeof preset.name === "string" ? preset.name.trim() : "";
  if (!name) return null;

  const rawColors = preset.colors;
  if (!rawColors || typeof rawColors !== "object") return null;

  const colors = normalizeCustomColors(rawColors as CustomColors);
  return { name, colors };
}

function applyColorsToDocument(colors: CustomColors) {
  const root = document.documentElement;
  for (const token of COLOR_TOKENS) {
    const value = colors[token.key];
    if (value) {
      root.style.setProperty(cssVar(token.key), value);
    } else {
      root.style.removeProperty(cssVar(token.key));
    }
  }
}

function slugify(value: string): string {
  return value
    .replace(/[^a-z0-9]+/gi, "_")
    .replace(/^_+|_+$/g, "")
    .toLowerCase();
}

const TOKEN_LABEL_KEYS: Record<ColorKey, { label: string; desc: string }> = {
  surface: { label: "colorCustomization.tokens.surface", desc: "colorCustomization.tokens.surfaceDesc" },
  surfaceEl: { label: "colorCustomization.tokens.surfaceEl", desc: "colorCustomization.tokens.surfaceElDesc" },
  nav: { label: "colorCustomization.tokens.nav", desc: "colorCustomization.tokens.navDesc" },
  fg: { label: "colorCustomization.tokens.foreground", desc: "colorCustomization.tokens.foregroundDesc" },
  accent: { label: "colorCustomization.tokens.accent", desc: "colorCustomization.tokens.accentDesc" },
  info: { label: "colorCustomization.tokens.info", desc: "colorCustomization.tokens.infoDesc" },
  warning: { label: "colorCustomization.tokens.warning", desc: "colorCustomization.tokens.warningDesc" },
  danger: { label: "colorCustomization.tokens.danger", desc: "colorCustomization.tokens.dangerDesc" },
  secondary: { label: "colorCustomization.tokens.secondary", desc: "colorCustomization.tokens.secondaryDesc" },
};

const GROUP_LABEL_KEYS: Record<string, string> = {
  backgrounds: "colorCustomization.groups.backgrounds",
  content: "colorCustomization.groups.content",
  semantic: "colorCustomization.groups.semantic",
};

const PRESET_NAME_KEYS: Record<string, string> = {
  "Default Dark": "colorCustomization.presets.defaultDark",
  "Midnight Blue": "colorCustomization.presets.midnightBlue",
  "Warm Earth": "colorCustomization.presets.warmEarth",
  "Purple Haze": "colorCustomization.presets.purpleHaze",
  "Rose Pine": "colorCustomization.presets.rosePine",
  "Tokyo Night": "colorCustomization.presets.tokyoNight",
  "Catppuccin": "colorCustomization.presets.catppuccin",
  "Gruvbox": "colorCustomization.presets.gruvbox",
  "Nord": "colorCustomization.presets.nord",
  "Dracula": "colorCustomization.presets.dracula",
  "Solarized": "colorCustomization.presets.solarized",
  "Ayu Dark": "colorCustomization.presets.ayuDark",
  "One Dark": "colorCustomization.presets.oneDark",
  "Vesper": "colorCustomization.presets.vesper",
  "Cyber Neon": "colorCustomization.presets.cyberNeon",
  "Monochrome": "colorCustomization.presets.monochrome",
};

export function ColorCustomizationPage() {
  const { setCustomColors: setThemeCustomColors } = useTheme();
  const { t } = useI18n();
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [workingColors, setWorkingColors] = useState<CustomColors>({});
  const [initialColors, setInitialColors] = useState<CustomColors>({});
  const [importedPresets, setImportedPresets] = useState<CustomColorPreset[]>([]);
  const [initialImportedPresets, setInitialImportedPresets] = useState<CustomColorPreset[]>([]);
  const [appliedPresetName, setAppliedPresetName] = useState<string | null>(null);
  const [derivedPresetId, setDerivedPresetId] = useState<string | null>(null);
  const pickerRefs = useRef<Record<string, HTMLInputElement | null>>({});
  const importInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const load = async () => {
      try {
        const [storedColors, storedPresets] = await Promise.all([
          getCustomColors(),
          getCustomColorPresets(),
        ]);
        const normalizedColors = normalizeCustomColors(storedColors ?? {});
        const normalizedPresets = (storedPresets ?? []).map(normalizePreset);

        setWorkingColors(normalizedColors);
        setInitialColors(normalizedColors);
        setImportedPresets(normalizedPresets);
        setInitialImportedPresets(normalizedPresets);
        applyColorsToDocument(normalizedColors);
      } catch (error) {
        console.error("Failed to load color customization settings:", error);
        toast.error("Load failed", "Could not load saved color settings.");
      } finally {
        setIsLoading(false);
      }
    };

    void load();
  }, []);

  useEffect(() => {
    if (isLoading) return;
    applyColorsToDocument(workingColors);
  }, [workingColors, isLoading]);

  const isDirty = useMemo(() => {
    return (
      !colorsEqual(workingColors, initialColors) ||
      !presetsEqual(importedPresets, initialImportedPresets)
    );
  }, [workingColors, initialColors, importedPresets, initialImportedPresets]);

  const getEffective = (key: ColorKey): string => workingColors[key] ?? "";

  const getDraftOrValue = (key: ColorKey): string => {
    if (key in drafts) return drafts[key];
    return getEffective(key);
  };

  const getDisplayColor = (key: ColorKey): string => {
    const v = getDraftOrValue(key);
    return isValidHex(v) ? v : DEFAULTS[key];
  };

  const getUniqueModifiedPresetName = useCallback(
    (parentName: string): string => {
      const base = `modified ${parentName}`;
      const exists = (candidate: string) => {
        const lower = candidate.toLowerCase();
        return (
          PRESETS.some((preset) => preset.name.toLowerCase() === lower) ||
          importedPresets.some((preset) => preset.name.toLowerCase() === lower)
        );
      };

      if (!exists(base)) return base;
      let index = 2;
      while (exists(`${base} #${index}`)) {
        index += 1;
      }
      return `${base} #${index}`;
    },
    [importedPresets],
  );

  const applyManualEdit = useCallback(
    (next: CustomColors) => {
      const normalized = normalizeCustomColors(next);
      if (colorsEqual(normalized, workingColors)) return;

      if (appliedPresetName) {
        if (derivedPresetId) {
          setImportedPresets((prev) =>
            prev.map((preset) =>
              preset.id === derivedPresetId ? { ...preset, colors: normalized } : preset,
            ),
          );
        } else {
          const createdPreset: CustomColorPreset = {
            id: crypto.randomUUID(),
            name: getUniqueModifiedPresetName(appliedPresetName),
            colors: normalized,
            createdAt: Date.now(),
          };
          setImportedPresets((prev) => [...prev, createdPreset]);
          setDerivedPresetId(createdPreset.id);
        }
      }

      setWorkingColors(normalized);
    },
    [appliedPresetName, derivedPresetId, getUniqueModifiedPresetName, workingColors],
  );

  const handleChange = useCallback(
    (key: ColorKey, value: string) => {
      setDrafts((prev) => ({ ...prev, [key]: value }));
      if (isValidHex(value)) {
        applyManualEdit({ ...workingColors, [key]: value });
      }
    },
    [applyManualEdit, workingColors],
  );

  const handleBlur = useCallback((key: ColorKey) => {
    setDrafts((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
  }, []);

  const handleReset = useCallback(
    (key: ColorKey) => {
      const next = { ...workingColors };
      delete next[key];
      applyManualEdit(next);
      setDrafts((prev) => {
        const next = { ...prev };
        delete next[key];
        return next;
      });
    },
    [applyManualEdit, workingColors],
  );

  const handleResetAll = useCallback(() => {
    applyManualEdit({});
    setDrafts({});
  }, [applyManualEdit]);

  const applyPreset = useCallback(
    (preset: { name: string; colors: Record<ColorKey, string> | CustomColors }) => {
      setWorkingColors(normalizeCustomColors(preset.colors));
      setDrafts({});
      setAppliedPresetName(preset.name);
      setDerivedPresetId(null);
    },
    [],
  );

  const handleSave = useCallback(async () => {
    if (!isDirty || isSaving) return;
    setIsSaving(true);
    try {
      const normalizedColors = normalizeCustomColors(workingColors);
      const normalizedPresets = importedPresets.map(normalizePreset);
      await persistCustomColors(normalizedColors);
      await setCustomColorPresets(normalizedPresets);
      setThemeCustomColors(normalizedColors);
      setInitialColors(normalizedColors);
      setInitialImportedPresets(normalizedPresets);
      toast.success("Saved", "Color customization updated.");
    } catch (error) {
      console.error("Failed to save color customization:", error);
      toast.error("Save failed", error instanceof Error ? error.message : String(error));
    } finally {
      setIsSaving(false);
    }
  }, [
    isDirty,
    isSaving,
    workingColors,
    importedPresets,
    setThemeCustomColors,
    persistCustomColors,
  ]);

  const handleDiscard = useCallback(() => {
    if (!isDirty) return;
    setWorkingColors(initialColors);
    setImportedPresets(initialImportedPresets);
    setDrafts({});
    setAppliedPresetName(null);
    setDerivedPresetId(null);
  }, [isDirty, initialColors, initialImportedPresets]);

  const handleImportJson = useCallback((rawJson: string) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(rawJson);
    } catch {
      toast.error("Import failed", "Invalid JSON file.");
      return;
    }

    const imported = parseImportedPreset(parsed);
    if (!imported) {
      toast.error(
        "Import failed",
        "Expected `{ name, colors }` or `{ preset: { name, colors } }`.",
      );
      return;
    }

    if (Object.keys(imported.colors).length === 0) {
      toast.error("Import failed", "No valid hex colors found in file.");
      return;
    }

    const createdAt = Date.now();
    const preset: CustomColorPreset = {
      id: crypto.randomUUID(),
      name: imported.name,
      colors: imported.colors,
      createdAt,
    };

    setImportedPresets((prev) => [...prev, preset]);
    toast.success("Imported", `Added preset \"${preset.name}\".`);
  }, []);

  const handleImportClick = useCallback(() => {
    importInputRef.current?.click();
  }, []);

  const handleExport = useCallback(() => {
    const defaultName = `Custom Preset ${new Date().toISOString().slice(0, 10)}`;
    const name = window.prompt("Export preset name", defaultName)?.trim();
    if (!name) return;

    const payload: PresetExportPayload = {
      type: "color-preset",
      version: 1,
      preset: {
        name,
        colors: buildPresetColors(workingColors),
      },
    };

    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    const date = new Date().toISOString().slice(0, 10);
    link.href = url;
    link.download = `color_preset_${slugify(name) || "export"}_${date}.json`;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);
  }, [workingColors]);

  const handleRenameImportedPreset = useCallback((preset: CustomColorPreset) => {
    const nextName = window.prompt("Rename preset", preset.name)?.trim();
    if (!nextName || nextName === preset.name) return;

    setImportedPresets((prev) =>
      prev.map((item) => (item.id === preset.id ? { ...item, name: nextName } : item)),
    );
  }, []);

  const handleDeleteImportedPreset = useCallback(
    (preset: CustomColorPreset) => {
      const confirmed = window.confirm(`Delete imported preset \"${preset.name}\"?`);
      if (!confirmed) return;

      setImportedPresets((prev) => prev.filter((item) => item.id !== preset.id));
      if (derivedPresetId === preset.id) {
        setDerivedPresetId(null);
        setAppliedPresetName(null);
      }
    },
    [derivedPresetId],
  );

  useEffect(() => {
    const handleDiscardEvent = () => {
      handleDiscard();
    };
    window.addEventListener("unsaved:discard", handleDiscardEvent);
    return () => window.removeEventListener("unsaved:discard", handleDiscardEvent);
  }, [handleDiscard]);

  useEffect(() => {
    const globalWindow = window as any;
    globalWindow.__saveColorCustomization = () => {
      void handleSave();
    };
    globalWindow.__saveColorCustomizationCanSave = isDirty;
    globalWindow.__saveColorCustomizationSaving = isSaving;
    return () => {
      delete globalWindow.__saveColorCustomization;
      delete globalWindow.__saveColorCustomizationCanSave;
      delete globalWindow.__saveColorCustomizationSaving;
    };
  }, [handleSave, isDirty, isSaving]);

  const hasAnyCustom = COLOR_TOKENS.some((tok) => workingColors[tok.key]);

  const isPresetActive = (preset: Preset | CustomColorPreset) => {
    const presetColors = "createdAt" in preset ? preset.colors : preset.colors;
    return COLOR_TOKENS.every((tok) => {
      const current = workingColors[tok.key];
      return (
        current === presetColors[tok.key] || (!current && presetColors[tok.key] === DEFAULTS[tok.key])
      );
    });
  };

  if (isLoading) return null;

  return (
    <div className="flex h-full flex-col pb-16">
      <section className="flex-1 min-h-0 overflow-y-auto px-3 pt-3 space-y-5">
        <input
          ref={importInputRef}
          type="file"
          accept="application/json,.json"
          className="hidden"
          onChange={async (event) => {
            const file = event.currentTarget.files?.[0];
            if (!file) return;
            try {
              const raw = await file.text();
              handleImportJson(raw);
            } catch (error) {
              console.error("Failed to import color preset:", error);
              toast.error("Import failed", "Could not read selected file.");
            } finally {
              event.currentTarget.value = "";
            }
          }}
        />

        {/* Presets */}
        <div>
          <div className="mb-2.5 flex items-center justify-between px-1 gap-2">
            <h2 className="text-[10px] font-semibold uppercase tracking-[0.25em] text-fg/35">
              {t("colorCustomization.presetsLabel")}
            </h2>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={handleImportClick}
                className={cn(
                  "flex items-center gap-1.5 px-2.5 py-1 text-[10px] font-medium text-fg/55",
                  radius.full,
                  "border border-fg/10 bg-fg/5",
                  interactive.transition.fast,
                  "hover:border-fg/20 hover:text-fg/75",
                )}
              >
                <Upload className="h-3 w-3" />
                {t("colorCustomization.importButton")}
              </button>
              <button
                type="button"
                onClick={handleExport}
                className={cn(
                  "flex items-center gap-1.5 px-2.5 py-1 text-[10px] font-medium text-fg/55",
                  radius.full,
                  "border border-fg/10 bg-fg/5",
                  interactive.transition.fast,
                  "hover:border-fg/20 hover:text-fg/75",
                )}
              >
                <Download className="h-3 w-3" />
                {t("colorCustomization.exportButton")}
              </button>
              {hasAnyCustom && (
                <button
                  type="button"
                  onClick={handleResetAll}
                  className={cn(
                    "flex items-center gap-1.5 px-2.5 py-1 text-[10px] font-medium text-fg/55",
                    radius.full,
                    "border border-fg/10 bg-fg/5",
                    interactive.transition.fast,
                    "hover:border-fg/20 hover:text-fg/75",
                  )}
                >
                  <RotateCcw className="h-3 w-3" />
                  {t("colorCustomization.resetAllButton")}
                </button>
              )}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            {PRESETS.map((preset) => {
              const active = isPresetActive(preset);
              return (
                <button
                  key={preset.name}
                  type="button"
                  onClick={() => applyPreset({ name: preset.name, colors: preset.colors })}
                  className={cn(
                    "flex items-center gap-2.5 rounded-lg border px-3 py-2",
                    interactive.transition.fast,
                    active
                      ? "border-accent/40 bg-accent/10"
                      : "border-fg/10 bg-fg/5 hover:border-fg/20 hover:bg-fg/8",
                  )}
                >
                  <div className="flex gap-0.5 shrink-0">
                    {(["accent", "info", "warning", "danger"] as const).map((k) => (
                      <div
                        key={k}
                        className="h-3 w-3 rounded-full"
                        style={{ backgroundColor: preset.colors[k] }}
                      />
                    ))}
                  </div>
                  <span
                    className={cn(
                      "text-[11px] font-medium truncate",
                      active ? "text-accent" : "text-fg/60",
                    )}
                  >
                    {PRESET_NAME_KEYS[preset.name] ? t(PRESET_NAME_KEYS[preset.name] as any) : preset.name}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        {importedPresets.length > 0 && (
          <div>
            <h2 className="mb-2 px-1 text-[10px] font-semibold uppercase tracking-[0.25em] text-fg/35">
              {t("colorCustomization.customPresetsLabel")}
            </h2>
            <div className="space-y-2">
              {importedPresets.map((preset) => {
                const active = isPresetActive(preset);
                return (
                  <div
                    key={preset.id}
                    className={cn(
                      "flex items-center gap-2 rounded-lg border px-3 py-2",
                      active ? "border-accent/40 bg-accent/10" : "border-fg/10 bg-fg/5",
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => applyPreset({ name: preset.name, colors: preset.colors })}
                      className="flex min-w-0 flex-1 items-center gap-2.5"
                    >
                      <div className="flex gap-1 shrink-0">
                        {(["accent", "info", "warning", "danger"] as const).map((k) => (
                          <div
                            key={k}
                            className="h-3 w-3 rounded-full"
                            style={{ backgroundColor: preset.colors[k] ?? DEFAULTS[k] }}
                          />
                        ))}
                      </div>
                      <span
                        className={cn(
                          "text-[11px] font-medium truncate",
                          active ? "text-accent" : "text-fg/60",
                        )}
                      >
                        {preset.name}
                      </span>
                    </button>
                    <button
                      type="button"
                      onClick={() => handleRenameImportedPreset(preset)}
                      className="text-fg/35 hover:text-fg/70 p-1"
                      title="Rename preset"
                    >
                      <Edit3 className="h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDeleteImportedPreset(preset)}
                      className="text-fg/35 hover:text-danger p-1 ml-1.5"
                      title="Delete preset"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Grouped token editors */}
        {TOKEN_GROUPS.map((group) => {
          const tokens = COLOR_TOKENS.filter((tok) => tok.group === group.id);
          return (
            <div key={group.id}>
              <h2 className="mb-2 px-1 text-[10px] font-semibold uppercase tracking-[0.25em] text-fg/35">
                {t(GROUP_LABEL_KEYS[group.id] as any)}
              </h2>
              <div className="space-y-2.5">
                {tokens.map((token) => {
                  const displayColor = getDisplayColor(token.key);
                  const inputValue = getDraftOrValue(token.key) || DEFAULTS[token.key];
                  const isCustom = Boolean(workingColors[token.key]);

                  return (
                    <div
                      key={token.key}
                      className={cn(
                        "rounded-xl border px-4 py-3",
                        isCustom ? "border-accent/25 bg-fg/6" : "border-fg/10 bg-fg/5",
                      )}
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div className="flex items-center gap-3">
                          <button
                            type="button"
                            className="relative h-8 w-8 shrink-0 rounded-full border border-fg/15 overflow-hidden"
                            style={{ backgroundColor: displayColor }}
                            onClick={() => pickerRefs.current[token.key]?.click()}
                          >
                            <input
                              ref={(el) => {
                                pickerRefs.current[token.key] = el;
                              }}
                              type="color"
                              value={displayColor}
                              onChange={(e) => handleChange(token.key, e.target.value)}
                              className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
                              tabIndex={-1}
                            />
                          </button>
                          <div>
                            <div className="text-sm font-medium text-fg">{t(TOKEN_LABEL_KEYS[token.key].label as any)}</div>
                            <div className="text-[11px] text-fg/45">{t(TOKEN_LABEL_KEYS[token.key].desc as any)}</div>
                          </div>
                        </div>

                        <div className="flex items-center gap-2">
                          <input
                            type="text"
                            value={inputValue}
                            onChange={(e) => handleChange(token.key, e.target.value)}
                            onBlur={() => handleBlur(token.key)}
                            placeholder={DEFAULTS[token.key]}
                            spellCheck={false}
                            className={cn(
                              "w-22.5 rounded-lg border px-2.5 py-1.5 font-mono text-xs text-fg",
                              "border-fg/10 bg-fg/5 placeholder-fg/30",
                              "focus:border-accent/40 focus:outline-none",
                              interactive.transition.fast,
                            )}
                          />
                          {isCustom && (
                            <button
                              type="button"
                              onClick={() => handleReset(token.key)}
                              className="text-fg/30 hover:text-fg/60"
                              title="Reset to default"
                            >
                              <RotateCcw className="h-3.5 w-3.5" />
                            </button>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}

        {/* Live preview */}
        <div>
          <h2 className="mb-2 px-1 text-[10px] font-semibold uppercase tracking-[0.25em] text-fg/35">
            {t("colorCustomization.previewLabel")}
          </h2>
          <div className="rounded-xl border border-fg/10 bg-surface p-4 space-y-4">
            <div className="space-y-1">
              <p className="text-sm font-medium text-fg">Primary text</p>
              <p className="text-xs text-fg/60">Secondary text at 60% opacity</p>
              <p className="text-[11px] text-fg/35">Tertiary text at 35% opacity</p>
            </div>

            <div className="flex flex-wrap gap-2">
              <span className="rounded-full border border-accent/40 bg-accent/15 px-3 py-1 text-[11px] font-medium text-accent">
                Accent
              </span>
              <span className="rounded-full border border-info/40 bg-info/15 px-3 py-1 text-[11px] font-medium text-info">
                Info
              </span>
              <span className="rounded-full border border-warning/40 bg-warning/15 px-3 py-1 text-[11px] font-medium text-warning">
                Warning
              </span>
              <span className="rounded-full border border-danger/40 bg-danger/15 px-3 py-1 text-[11px] font-medium text-danger">
                Danger
              </span>
            </div>

            <div className="rounded-lg border border-fg/10 bg-surface-el p-3 space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-fg">Sample Card</span>
                <span className="rounded-full bg-accent/15 px-2 py-0.5 text-[10px] font-medium text-accent">
                  Active
                </span>
              </div>
              <p className="text-[11px] text-fg/50">
                This card uses the elevated surface background with foreground text.
              </p>
              <div className="flex gap-2">
                <div className="flex-1 rounded-md bg-accent/10 px-2 py-1.5 text-center text-[10px] font-medium text-accent">
                  Confirm
                </div>
                <div className="flex-1 rounded-md bg-danger/10 px-2 py-1.5 text-center text-[10px] font-medium text-danger">
                  Delete
                </div>
              </div>
            </div>

            <div className="flex items-center justify-between rounded-lg border border-fg/10 bg-surface-el px-3 py-2.5">
              <span className="text-xs text-fg/70">Sample toggle</span>
              <div className="relative inline-flex h-6 w-11 rounded-full bg-accent">
                <span className="inline-block h-5 w-5 translate-x-5 transform rounded-full bg-fg transition" />
              </div>
            </div>
          </div>
        </div>

        <div className="h-4" />
      </section>
    </div>
  );
}
