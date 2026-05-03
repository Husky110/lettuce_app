import { useState, useEffect, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { motion, AnimatePresence } from "framer-motion";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
} from "recharts";
import {
  useUsageTracking,
  RequestUsage,
  UsageFilter,
  AppActiveUsageSummary,
} from "../../../core/usage";
import {
  Download,
  TrendingUp,
  TrendingDown,
  Zap,
  DollarSign,
  Activity,
  Clock,
  Filter,
  ChevronRight,
  Calendar,
} from "lucide-react";
import { BottomMenu } from "../../components";
import { useI18n } from "../../../core/i18n/context";
import { ActivityItem, formatCompactNumber, formatCurrency } from "./UsageActivityShared";
import { getEffectiveTotalCost } from "./UsageActivityShared";
import { typography, colors, components, cn, animations, radius } from "../../design-tokens";

// ============================================================================
// Utilities
// ============================================================================

function formatNumber(value: number): string {
  if (value === 0) return "0";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return value.toLocaleString();
}

function formatDurationMs(durationMs: number): string {
  if (durationMs <= 0) return "0s";
  const totalSeconds = Math.floor(durationMs / 1000);
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  if (seconds > 0) return `${seconds}s`;
  return `${minutes}m`;
}

function dayKeyFromDate(date: Date): string {
  const y = date.getFullYear();
  const m = `${date.getMonth() + 1}`.padStart(2, "0");
  const d = `${date.getDate()}`.padStart(2, "0");
  return `${y}-${m}-${d}`;
}

// ============================================================================
// Date Range Presets
// ============================================================================

type DatePreset = "today" | "week" | "month" | "all" | "custom";

function getDateRange(preset: DatePreset): { start: Date; end: Date } {
  const now = new Date();
  const end = new Date(now);
  end.setHours(23, 59, 59, 999);

  const start = new Date(now);
  start.setHours(0, 0, 0, 0);

  switch (preset) {
    case "today":
      break;
    case "week":
      start.setDate(start.getDate() - 6);
      break;
    case "month":
      start.setDate(start.getDate() - 29);
      break;
    case "all":
      start.setFullYear(start.getFullYear() - 10);
      break;
    case "custom":
      break;
  }

  return { start, end };
}

// ============================================================================
// Chart Tooltip
// ============================================================================

const ChartTooltip = ({ active, payload, label }: any) => {
  if (!active || !payload?.length) return null;

  return (
    <div className={cn(
      "max-w-[70vw] px-3 py-2.5 shadow-xl",
      components.card.base,
      colors.glass.default
    )}>
      <p className={cn(typography.overline.size, "text-fg/50 mb-2")}>{label}</p>
      <div className="space-y-1.5">
        {payload.map((p: any, i: number) => (
          <div key={i} className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-2">
              <span className="h-2 w-2 rounded-full" style={{ backgroundColor: p.color }} />
              <span className={cn(typography.caption.size, "text-fg/60")}>{p.name}</span>
            </div>
            <span className={cn(typography.caption.size, typography.h3.weight, "text-fg")}>
              {formatNumber(p.value)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
};

const AppTimeTooltip = ({ active, payload, label }: any) => {
  if (!active || !payload?.length) return null;

  return (
    <div className={cn(
      "max-w-[70vw] px-3 py-2.5 shadow-xl",
      components.card.base,
      colors.glass.default
    )}>
      <p className={cn(typography.overline.size, "text-fg/50 mb-2")}>{label}</p>
      {payload.map((p: any, i: number) => (
        <div key={i} className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2">
            <span className="h-2 w-2 rounded-full" style={{ backgroundColor: p.color }} />
            <span className={cn(typography.caption.size, "text-fg/60")}>{p.name}</span>
          </div>
          <span className={cn(typography.caption.size, typography.h3.weight, "text-fg")}>
            {formatDurationMs(p.value)}
          </span>
        </div>
      ))}
    </div>
  );
};

// ============================================================================
// Stat Card
// ============================================================================

function StatCard({
  icon: Icon,
  label,
  value,
  subValue,
  trend,
  highlight,
}: {
  icon: any;
  label: string;
  value: string;
  subValue?: string;
  trend?: { value: number; isUp: boolean } | null;
  highlight?: boolean;
}) {
  return (
    <div
      className={cn(
        "relative overflow-hidden p-4",
        components.card.base,
        highlight 
          ? "bg-accent/10 border-accent/25" 
          : "bg-fg/5 border-fg/10"
      )}
    >
      {highlight && (
        <div className="absolute top-0 right-0 -mr-8 -mt-8 h-24 w-24 rounded-full bg-accent/5 blur-2xl" />
      )}
      
      <div className="relative z-10">
        <div className="flex items-center gap-2 mb-3">
          <div className={cn(
            "p-1.5 rounded-lg",
            highlight ? "bg-accent/20 text-accent" : "bg-fg/5 text-fg/40"
          )}>
            <Icon className="h-3.5 w-3.5" />
          </div>
          <span className={cn(
            typography.overline.size,
            typography.overline.weight,
            typography.overline.tracking,
            typography.overline.transform,
            "text-fg/40"
          )}>
            {label}
          </span>
        </div>
        
        <div className="flex items-end justify-between gap-2">
          <div className="min-w-0">
            <p className={cn(
              typography.display.size,
              typography.display.weight,
              typography.display.tracking,
              highlight ? "text-accent" : "text-fg"
            )}>
              {value}
            </p>
            {subValue && (
              <p className={cn(typography.caption.size, "text-fg/30 mt-0.5 truncate")}>
                {subValue}
              </p>
            )}
          </div>
          
          {trend && trend.value > 0 && (
            <div
              className={cn(
                "flex items-center gap-0.5 rounded-full px-2 py-0.5 mb-1 text-[10px] font-bold",
                trend.isUp ? "bg-accent/15 text-accent" : "bg-danger/15 text-danger"
              )}
            >
              {trend.isUp ? <TrendingUp className="h-3 w-3" /> : <TrendingDown className="h-3 w-3" />}
              {trend.value.toFixed(0)}%
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// Main Component
// ============================================================================

export function UsagePage() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const { queryRecords, exportCSV, saveCSV, getAppActiveUsage } = useUsageTracking();
  const [appActiveUsage, setAppActiveUsage] = useState<AppActiveUsageSummary>({
    totalMs: 0,
    byDayMs: {},
  });

  // View mode
  const [viewMode, setViewMode] = useState<"dashboard" | "appTime">("dashboard");

  // Dashboard state
  const [datePreset, setDatePreset] = useState<DatePreset>("month");
  const [records, setRecords] = useState<RequestUsage[]>([]);
  const [, setLoading] = useState(true);
  const [showFilters, setShowFilters] = useState(false);
  const [showAllActivity, setShowAllActivity] = useState(false);
  const [exporting, setExporting] = useState(false);

  // Custom date range
  const [customStartDate, setCustomStartDate] = useState<Date>(() => {
    const d = new Date();
    d.setDate(d.getDate() - 30);
    d.setHours(0, 0, 0, 0);
    return d;
  });
  const [customEndDate, setCustomEndDate] = useState<Date>(() => {
    const d = new Date();
    d.setHours(23, 59, 59, 999);
    return d;
  });
  const [showCustomDatePicker, setShowCustomDatePicker] = useState(false);
  const [appTimePreset, setAppTimePreset] = useState<"today" | "week" | "month" | "all" | "custom">(
    "week",
  );

  // Filters
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [selectedCharacter, setSelectedCharacter] = useState<string | null>(null);

  const resolveDashboardRange = () => {
    if (datePreset === "custom") {
      return { start: customStartDate, end: customEndDate };
    }
    return getDateRange(datePreset);
  };

  // Load dashboard data
  const loadDashboardData = async () => {
    setLoading(true);
    const { start, end } = resolveDashboardRange();

    const filter: UsageFilter = {
      startTimestamp: start.getTime(),
      endTimestamp: end.getTime(),
    };
    const newRecords = await queryRecords(filter);
    if (newRecords) setRecords(newRecords);
    setLoading(false);
  };

  const loadAppUsageInfo = async () => {
    const summary = await getAppActiveUsage();
    if (!summary) return;
    setAppActiveUsage(summary);
  };

  useEffect(() => {
    if (viewMode === "dashboard") {
      loadDashboardData();
    }
  }, [datePreset, viewMode, customStartDate, customEndDate]);

  useEffect(() => {
    loadAppUsageInfo();
  }, []);

  useEffect(() => {
    if (viewMode !== "appTime") return;
    loadAppUsageInfo();
    const interval = window.setInterval(() => {
      loadAppUsageInfo();
    }, 15000);
    return () => window.clearInterval(interval);
  }, [viewMode]);

  // Filtered records for dashboard
  const filteredRecords = useMemo(() => {
    let list = [...records];
    if (selectedModel) list = list.filter((r) => r.modelId === selectedModel);
    if (selectedCharacter) list = list.filter((r) => r.characterId === selectedCharacter);
    return list.sort((a, b) => b.timestamp - a.timestamp);
  }, [records, selectedModel, selectedCharacter]);

  // Dashboard derived data
  const { modelOptions, characterOptions, chartData, topModels } = useMemo(() => {
    const modelMap = new Map<string, { name: string; tokens: number; cost: number }>();
    const charMap = new Map<string, { name: string; tokens: number; cost: number }>();
    const dailyMap = new Map<string, { input: number; output: number; cost: number; date: Date }>();

    for (const r of filteredRecords) {
      if (r.modelId && r.modelName) {
        const existing = modelMap.get(r.modelId) || { name: r.modelName, tokens: 0, cost: 0 };
        existing.tokens += r.totalTokens || 0;
        existing.cost += getEffectiveTotalCost(r);
        modelMap.set(r.modelId, existing);
      }
      if (r.characterId && r.characterName) {
        const existing = charMap.get(r.characterId) || {
          name: r.characterName,
          tokens: 0,
          cost: 0,
        };
        existing.tokens += r.totalTokens || 0;
        existing.cost += getEffectiveTotalCost(r);
        charMap.set(r.characterId, existing);
      }
      const recordDate = new Date(r.timestamp);
      const dateKey = dayKeyFromDate(recordDate);
      const dayData = dailyMap.get(dateKey) || { input: 0, output: 0, cost: 0, date: recordDate };
      dayData.input += r.promptTokens || 0;
      dayData.output += r.completionTokens || 0;
      dayData.cost += getEffectiveTotalCost(r);
      dailyMap.set(dateKey, dayData);
    }

    const modelOptions = Array.from(modelMap.entries())
      .map(([id, data]) => ({ id, ...data }))
      .sort((a, b) => b.tokens - a.tokens);

    const characterOptions = Array.from(charMap.entries())
      .map(([id, data]) => ({ id, ...data }))
      .sort((a, b) => b.tokens - a.tokens);

    const daysToShow =
      datePreset === "today"
        ? 1
        : datePreset === "week"
          ? 7
          : datePreset === "month"
            ? 30
            : undefined;

    const sortedEntries = Array.from(dailyMap.entries()).sort(
      (a, b) => a[1].date.getTime() - b[1].date.getTime(),
    );

    const chartData = (
      daysToShow !== undefined ? sortedEntries.slice(-daysToShow) : sortedEntries
    ).map(([, data]) => ({
      label: data.date.toLocaleDateString(undefined, { month: "short", day: "numeric" }),
      input: data.input,
      output: data.output,
      cost: data.cost,
    }));

    const topModels = modelOptions.slice(0, 5);

    return { modelOptions, characterOptions, chartData, topModels };
  }, [filteredRecords, datePreset]);

  // Dashboard stats
  const displayStats = useMemo(() => {
    const totals = filteredRecords.reduce(
      (acc, r) => {
        acc.tokens += r.totalTokens || 0;
        acc.cost += getEffectiveTotalCost(r);
        acc.requests += 1;
        return acc;
      },
      { tokens: 0, cost: 0, requests: 0 },
    );
    return {
      ...totals,
      avgPerRequest: totals.requests > 0 ? totals.tokens / totals.requests : 0,
    };
  }, [filteredRecords]);

  const appTimeStats = useMemo(() => {
    const byDay = appActiveUsage.byDayMs ?? {};
    const today = new Date();
    const sumPreviousDays = (startOffsetDays: number, days: number) => {
      let sum = 0;
      for (let i = startOffsetDays; i < startOffsetDays + days; i += 1) {
        const d = new Date(today);
        d.setDate(today.getDate() - i);
        const key = dayKeyFromDate(d);
        sum += byDay[key] ?? 0;
      }
      return sum;
    };
    const todayMs = sumPreviousDays(0, 1);
    const yesterdayMs = sumPreviousDays(1, 1);
    const avg30Ms = sumPreviousDays(1, 30) / 30;

    const sumRangeFromDate = (start: Date, days: number) => {
      let sum = 0;
      const d = new Date(start);
      d.setHours(0, 0, 0, 0);
      for (let i = 0; i < days; i += 1) {
        const key = dayKeyFromDate(d);
        sum += byDay[key] ?? 0;
        d.setDate(d.getDate() + 1);
      }
      return sum;
    };

    const customRangeKeys = (() => {
      const keys: string[] = [];
      const d = new Date(customStartDate);
      d.setHours(0, 0, 0, 0);
      const end = new Date(customEndDate);
      end.setHours(0, 0, 0, 0);
      while (d <= end) {
        keys.push(dayKeyFromDate(d));
        d.setDate(d.getDate() + 1);
      }
      return keys;
    })();

    const daysToShow =
      appTimePreset === "today"
        ? 1
        : appTimePreset === "week"
          ? 7
          : appTimePreset === "month"
            ? 30
            : appTimePreset === "custom"
              ? customRangeKeys.length
              : undefined;
    
    const allDayKeysSorted = Object.keys(byDay).sort();
    const chartKeys =
      daysToShow === undefined
        ? allDayKeysSorted
        : appTimePreset === "custom"
          ? customRangeKeys
          : Array.from({ length: daysToShow }, (_, idx) => {
              const d = new Date(today);
              d.setDate(today.getDate() - (daysToShow - 1 - idx));
              return dayKeyFromDate(d);
            });

    const byDayChart = chartKeys.map((key) => {
      const d = key.includes("-") ? new Date(`${key}T00:00:00`) : new Date();
      return {
        label: d.toLocaleDateString(undefined, { month: "short", day: "numeric" }),
        ms: byDay[key] ?? 0,
      };
    });
    const rangeTotalMs = byDayChart.reduce((sum, item) => sum + item.ms, 0);
    const selectedDays = Math.max(byDayChart.length, 1);
    const dailyAvgInRangeMs = rangeTotalMs / selectedDays;
    const prevRangeTotalMs =
      daysToShow === undefined
        ? 0
        : appTimePreset === "custom"
          ? (() => {
              const prevStart = new Date(customStartDate);
              prevStart.setHours(0, 0, 0, 0);
              prevStart.setDate(prevStart.getDate() - selectedDays);
              return sumRangeFromDate(prevStart, selectedDays);
            })()
          : sumPreviousDays(daysToShow, daysToShow);
    const rangeDeltaMs = rangeTotalMs - prevRangeTotalMs;
    const rangeDeltaPct =
      prevRangeTotalMs > 0 ? (rangeDeltaMs / prevRangeTotalMs) * 100 : rangeTotalMs > 0 ? 100 : 0;

    return {
      todayMs,
      yesterdayMs,
      avg30Ms,
      rangeTotalMs,
      selectedDays,
      dailyAvgInRangeMs,
      prevRangeTotalMs,
      rangeDeltaMs,
      rangeDeltaPct,
      byDayChart,
    };
  }, [appActiveUsage, appTimePreset, customStartDate, customEndDate]);

  const handleExport = async () => {
    setExporting(true);
    try {
      const { start, end } = resolveDashboardRange();
      const csv = await exportCSV({
        startTimestamp: start.getTime(),
        endTimestamp: end.getTime(),
      });
      if (csv) {
        const fileName = `usage-${datePreset}-${new Date().toISOString().split("T")[0]}.csv`;
        const path = await saveCSV(csv, fileName);
        if (path) alert(t("usageAnalytics.page.exportedTo", { path }));
      }
    } catch (e) {
      console.error("Export failed:", e);
    } finally {
      setExporting(false);
    }
  };

  const COLORS = [
    "var(--color-accent)",
    "var(--color-info)",
    "var(--color-secondary)",
    "#f472b6",
    "var(--color-warning)",
  ];
  const activeFilterCount = [selectedModel, selectedCharacter].filter(Boolean).length;

  return (
    <div className="min-h-screen bg-surface pb-32">
      <BottomMenu
        isOpen={showFilters}
        onClose={() => setShowFilters(false)}
        title={t("usageAnalytics.page.filters")}
        includeExitIcon={false}
      >
        <div className="space-y-6 pb-8">
          <div>
            <label className={cn(
              typography.overline.size,
              typography.overline.weight,
              typography.overline.tracking,
              "text-fg/40 ml-1 mb-3 block"
            )}>
              {t("usageAnalytics.page.model")}
            </label>
            <div className="space-y-1.5 max-h-56 overflow-y-auto pr-1">
              {modelOptions.slice(0, 12).map((m) => (
                <button
                  key={m.id}
                  onClick={() => setSelectedModel(selectedModel === m.id ? null : m.id)}
                  className={cn(
                    "w-full flex items-center justify-between px-3.5 py-3 transition-all",
                    components.listItem.base,
                    selectedModel === m.id
                      ? "bg-accent/15 border-accent/30 text-accent"
                      : "bg-fg/5 text-fg/70 hover:bg-fg/10"
                  )}
                >
                  <span className={cn(typography.body.size, "truncate")}>{m.name}</span>
                  <span className={cn(typography.caption.size, "text-fg/40")}>{formatCompactNumber(m.tokens)}</span>
                </button>
              ))}
            </div>
          </div>

          <div>
            <label className={cn(
              typography.overline.size,
              typography.overline.weight,
              typography.overline.tracking,
              "text-fg/40 ml-1 mb-3 block"
            )}>
              {t("usageAnalytics.page.character")}
            </label>
            <div className="space-y-1.5 max-h-56 overflow-y-auto pr-1">
              {characterOptions.slice(0, 12).map((c) => (
                <button
                  key={c.id}
                  onClick={() => setSelectedCharacter(selectedCharacter === c.id ? null : c.id)}
                  className={cn(
                    "w-full flex items-center justify-between px-3.5 py-3 transition-all",
                    components.listItem.base,
                    selectedCharacter === c.id
                      ? "bg-accent/15 border-accent/30 text-accent"
                      : "bg-fg/5 text-fg/70 hover:bg-fg/10"
                  )}
                >
                  <span className={cn(typography.body.size, "truncate")}>{c.name}</span>
                  <span className={cn(typography.caption.size, "text-fg/40")}>{formatCompactNumber(c.tokens)}</span>
                </button>
              ))}
            </div>
          </div>

          <div className="flex gap-3 pt-2">
            <button
              onClick={() => {
                setSelectedModel(null);
                setSelectedCharacter(null);
              }}
              className={cn(
                "flex-1 py-3 border border-fg/10 text-fg/60 font-semibold transition-all",
                components.button.primary,
                "hover:bg-fg/5"
              )}
            >
              {t("usageAnalytics.page.clearAll")}
            </button>
            <button
              onClick={() => setShowFilters(false)}
              className={cn(
                "flex-1 py-3 bg-accent text-black font-bold transition-all",
                components.button.primary,
                "hover:brightness-110 shadow-lg shadow-accent/20"
              )}
            >
              {t("usageAnalytics.page.applyFilters")}
            </button>
          </div>
        </div>
      </BottomMenu>

      <BottomMenu
        isOpen={showAllActivity}
        onClose={() => setShowAllActivity(false)}
        title={t("usageAnalytics.page.recentActivity")}
        includeExitIcon={false}
      >
        <div className="space-y-0.5 pb-8 max-h-[65vh] overflow-y-auto pr-1">
          {filteredRecords.slice(0, 50).map((r) => (
            <ActivityItem key={r.id} request={r} />
          ))}
          {filteredRecords.length === 0 && (
            <div className="py-20 text-center">
               <Calendar className="h-10 w-10 text-fg/10 mx-auto mb-4" />
               <p className={cn(typography.body.size, "text-fg/40")}>{t("common.labels.none")}</p>
            </div>
          )}
        </div>
      </BottomMenu>

      <BottomMenu
        isOpen={showCustomDatePicker}
        onClose={() => setShowCustomDatePicker(false)}
        title={t("usageAnalytics.page.customRange")}
        includeExitIcon={false}
      >
        <div className="space-y-6 pb-8">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className={cn(typography.overline.size, "text-fg/40 mb-2 block")}>{t("usageAnalytics.page.startDate")}</label>
              <input
                type="date"
                value={customStartDate.toISOString().split("T")[0]}
                onChange={(e) => {
                  const newDate = new Date(e.target.value);
                  newDate.setHours(0, 0, 0, 0);
                  setCustomStartDate(newDate);
                }}
                max={customEndDate.toISOString().split("T")[0]}
                className={cn(
                  "w-full bg-fg/5 border border-fg/10 text-fg focus:border-accent/40 focus:bg-fg/10 transition-all",
                  components.input.base,
                  components.input.sizes.md
                )}
              />
            </div>
            <div>
              <label className={cn(typography.overline.size, "text-fg/40 mb-2 block")}>{t("usageAnalytics.page.endDate")}</label>
              <input
                type="date"
                value={customEndDate.toISOString().split("T")[0]}
                onChange={(e) => {
                  const newDate = new Date(e.target.value);
                  newDate.setHours(23, 59, 59, 999);
                  setCustomEndDate(newDate);
                }}
                min={customStartDate.toISOString().split("T")[0]}
                max={new Date().toISOString().split("T")[0]}
                className={cn(
                  "w-full bg-fg/5 border border-fg/10 text-fg focus:border-accent/40 focus:bg-fg/10 transition-all",
                  components.input.base,
                  components.input.sizes.md
                )}
              />
            </div>
          </div>

          <div className="flex gap-3 pt-2">
            <button
              onClick={() => setShowCustomDatePicker(false)}
              className={cn(
                "flex-1 py-3 border border-fg/10 text-fg/60 font-semibold transition-all",
                components.button.primary,
                "hover:bg-fg/5"
              )}
            >
              {t("common.buttons.cancel")}
            </button>
            <button
              onClick={() => {
                if (viewMode === "appTime") {
                  setAppTimePreset("custom");
                } else {
                  setDatePreset("custom");
                }
                setShowCustomDatePicker(false);
              }}
              className={cn(
                "flex-1 py-3 bg-accent text-black font-bold transition-all",
                components.button.primary,
                "hover:brightness-110"
              )}
            >
              {t("usageAnalytics.page.applyRange")}
            </button>
          </div>
        </div>
      </BottomMenu>

      <div className="max-w-4xl mx-auto px-4 py-6">
        <div className="flex flex-col gap-6">
          <div className="flex items-center justify-center">
            <div className={cn(
              "p-1 flex items-center gap-1 bg-fg/5 border border-fg/10",
              radius.lg
            )}>
              <button
                onClick={() => setViewMode("dashboard")}
                className={cn(
                  "flex items-center gap-2 px-6 py-2 rounded-lg transition-all duration-300",
                  typography.label.size,
                  typography.label.weight,
                  viewMode === "dashboard" 
                    ? "bg-fg text-surface shadow-lg" 
                    : "text-fg/40 hover:text-fg/70 hover:bg-fg/5"
                )}
              >
                <Activity className="h-3.5 w-3.5" />
                {t("usageAnalytics.page.dashboard")}
              </button>
              <button
                onClick={() => setViewMode("appTime")}
                className={cn(
                  "flex items-center gap-2 px-6 py-2 rounded-lg transition-all duration-300",
                  typography.label.size,
                  typography.label.weight,
                  viewMode === "appTime" 
                    ? "bg-fg text-surface shadow-lg" 
                    : "text-fg/40 hover:text-fg/70 hover:bg-fg/5"
                )}
              >
                <Clock className="h-3.5 w-3.5" />
                {t("usageAnalytics.page.appTime")}
              </button>
            </div>
          </div>

          <AnimatePresence mode="wait">
            {viewMode === "dashboard" ? (
              <motion.div
                key="dashboard"
                {...animations.fadeIn}
                className="space-y-6"
              >
                <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className={cn(
                    "flex flex-wrap items-center gap-1.5 p-1.5 bg-fg/5 border border-fg/10",
                    radius.lg
                  )}>
                    {[
                      { key: "today", label: t("usageAnalytics.page.today") },
                      { key: "week", label: t("usageAnalytics.page.last7Days") },
                      { key: "month", label: t("usageAnalytics.page.last30Days") },
                      { key: "all", label: t("usageAnalytics.page.all") },
                      { key: "custom", label: t("usageAnalytics.page.custom") },
                    ].map(({ key, label }) => (
                      <button
                        key={key}
                        onClick={() => {
                          if (key === "custom") {
                            setShowCustomDatePicker(true);
                            return;
                          }
                          setDatePreset(key as DatePreset);
                        }}
                        className={cn(
                          "px-4 py-1.5 rounded-lg transition-all duration-200",
                          typography.caption.size,
                          typography.caption.weight,
                          datePreset === key 
                            ? "bg-fg/10 text-fg shadow-sm" 
                            : "text-fg/40 hover:text-fg/70 hover:bg-fg/5"
                        )}
                      >
                        {label}
                      </button>
                    ))}
                  </div>

                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => setShowFilters(true)}
                      className={cn(
                        "flex items-center gap-2 px-4 py-2 border transition-all",
                        components.button.primary,
                        activeFilterCount > 0
                          ? "bg-accent/15 border-accent/40 text-accent shadow-sm"
                          : "bg-fg/5 border-fg/10 text-fg/50 hover:bg-fg/10"
                      )}
                    >
                      <Filter className="h-3.5 w-3.5" />
                      <span className={typography.caption.size}>
                        {activeFilterCount > 0
                          ? t("usageAnalytics.page.filtersCount", { count: activeFilterCount })
                          : t("usageAnalytics.page.filters")}
                      </span>
                    </button>
                    <button
                      onClick={handleExport}
                      disabled={exporting || records.length === 0}
                      className={cn(
                        "flex items-center justify-center p-2 border transition-all",
                        components.button.primary,
                        "bg-fg/5 border-fg/10 text-fg/50 hover:bg-fg/10 disabled:opacity-20"
                      )}
                    >
                      <Download className="h-4 w-4" />
                    </button>
                  </div>
                </div>

                <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
                  <StatCard
                    icon={DollarSign}
                    label={t("usageAnalytics.page.totalCost")}
                    value={formatCurrency(displayStats.cost)}
                    highlight
                  />
                  <StatCard
                    icon={Zap}
                    label={t("usageAnalytics.page.tokens")}
                    value={formatNumber(displayStats.tokens)}
                    subValue={t("usageAnalytics.page.avgShort", { value: formatNumber(Math.round(displayStats.avgPerRequest)) })}
                  />
                  <StatCard
                    icon={Activity}
                    label={t("usageAnalytics.page.requests")}
                    value={displayStats.requests.toLocaleString()}
                  />
                  <StatCard
                    icon={Clock}
                    label={t("usageAnalytics.page.period")}
                    value={
                      datePreset === "today" ? t("usageAnalytics.page.today") : 
                      datePreset === "week" ? t("usageAnalytics.page.last7d") : 
                      datePreset === "month" ? t("usageAnalytics.page.last30d") : 
                      datePreset === "custom" ? t("usageAnalytics.page.custom") : t("usageAnalytics.page.allTime")
                    }
                    subValue={t("usageAnalytics.page.recordsCount", { count: filteredRecords.length })}
                  />
                </div>

                {chartData.length > 1 && (
                  <div className={cn(
                    "p-6",
                    components.card.base,
                    "bg-fg/5 border-fg/10"
                  )}>
                    <div className="flex items-center justify-between mb-8">
                      <div>
                        <h3 className={cn(typography.h3.size, typography.h3.weight, "text-fg")}>{t("usageAnalytics.page.usageTrend")}</h3>
                        <p className={cn(typography.caption.size, "text-fg/40 mt-0.5")}>{t("usageAnalytics.page.tokenConsumptionOverTime")}</p>
                      </div>
                      <div className="flex items-center gap-4">
                        <div className="flex items-center gap-2">
                          <span className="h-2 w-2 rounded-full bg-info" />
                          <span className={cn(typography.overline.size, "text-fg/40")}>{t("usageAnalytics.page.input")}</span>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="h-2 w-2 rounded-full bg-accent" />
                          <span className={cn(typography.overline.size, "text-fg/40")}>{t("usageAnalytics.page.output")}</span>
                        </div>
                      </div>
                    </div>
                    <div className="h-64">
                      <ResponsiveContainer width="100%" height="100%">
                        <AreaChart
                          data={chartData}
                          margin={{ top: 0, right: 0, left: -20, bottom: 0 }}
                        >
                          <defs>
                            <linearGradient id="inputGrad" x1="0" y1="0" x2="0" y2="1">
                              <stop offset="5%" stopColor="var(--color-info)" stopOpacity={0.25} />
                              <stop offset="95%" stopColor="var(--color-info)" stopOpacity={0} />
                            </linearGradient>
                            <linearGradient id="outputGrad" x1="0" y1="0" x2="0" y2="1">
                              <stop offset="5%" stopColor="var(--color-accent)" stopOpacity={0.25} />
                              <stop offset="95%" stopColor="var(--color-accent)" stopOpacity={0} />
                            </linearGradient>
                          </defs>
                          <XAxis
                            dataKey="label"
                            tick={{ fill: "rgba(255,255,255,0.3)", fontSize: 10, fontWeight: 500 }}
                            axisLine={false}
                            tickLine={false}
                            dy={10}
                          />
                          <YAxis
                            tick={{ fill: "rgba(255,255,255,0.3)", fontSize: 10, fontWeight: 500 }}
                            axisLine={false}
                            tickLine={false}
                            tickFormatter={formatCompactNumber}
                          />
                          <Tooltip content={<ChartTooltip />} cursor={{ stroke: 'rgba(255,255,255,0.1)', strokeWidth: 1 }} />
                          <Area
                            type="monotone"
                            dataKey="input"
                            name={t("usageAnalytics.page.input")}
                            stroke="var(--color-info)"
                            fill="url(#inputGrad)"
                            strokeWidth={2.5}
                            animationDuration={1500}
                          />
                          <Area
                            type="monotone"
                            dataKey="output"
                            name={t("usageAnalytics.page.output")}
                            stroke="var(--color-accent)"
                            fill="url(#outputGrad)"
                            strokeWidth={2.5}
                            animationDuration={1500}
                          />
                        </AreaChart>
                      </ResponsiveContainer>
                    </div>
                  </div>
                )}

                <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                  {topModels.length > 0 && (
                    <div className={cn(
                      "p-5",
                      components.card.base,
                      "bg-fg/5 border-fg/10"
                    )}>
                      <h3 className={cn(typography.h3.size, typography.h3.weight, "text-fg mb-6")}>{t("usageAnalytics.page.byModel")}</h3>
                      <div className="flex items-center gap-6">
                        <div className="w-32 h-32 shrink-0 relative">
                           <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
                              <span className={cn(typography.caption.size, "text-fg/30 uppercase tracking-widest")}>{t("usageAnalytics.page.top")}</span>
                              <span className={cn(typography.h2.size, typography.h2.weight, "text-fg")}>{topModels.length}</span>
                           </div>
                          <ResponsiveContainer width="100%" height="100%">
                            <PieChart>
                              <Pie
                                data={topModels}
                                dataKey="tokens"
                                nameKey="name"
                                cx="50%"
                                cy="50%"
                                innerRadius={45}
                                outerRadius={60}
                                paddingAngle={4}
                                stroke="none"
                              >
                                {topModels.map((_, i) => (
                                  <Cell key={i} fill={COLORS[i % COLORS.length]} />
                                ))}
                              </Pie>
                            </PieChart>
                          </ResponsiveContainer>
                        </div>
                        <div className="flex-1 space-y-3">
                          {topModels.slice(0, 4).map((m, i) => (
                            <div key={m.id} className="flex items-center gap-3">
                              <div
                                className="h-2 w-2 rounded-full shrink-0"
                                style={{ backgroundColor: COLORS[i % COLORS.length] }}
                              />
                              <span className={cn(typography.bodySmall.size, "text-fg/70 truncate flex-1")}>{m.name}</span>
                              <span className={cn(typography.caption.size, "text-fg/40 font-bold")}>
                                {formatCompactNumber(m.tokens)}
                              </span>
                            </div>
                          ))}
                        </div>
                      </div>
                    </div>
                  )}

                  {characterOptions.length > 0 && (
                    <div className={cn(
                      "p-5",
                      components.card.base,
                      "bg-fg/5 border-fg/10"
                    )}>
                      <h3 className={cn(typography.h3.size, typography.h3.weight, "text-fg mb-6")}>{t("usageAnalytics.page.byCharacter")}</h3>
                      <div className="flex items-center gap-6">
                        <div className="w-32 h-32 shrink-0 relative">
                          <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
                              <span className={cn(typography.caption.size, "text-fg/30 uppercase tracking-widest")}>{t("usageAnalytics.page.active")}</span>
                              <span className={cn(typography.h2.size, typography.h2.weight, "text-fg")}>{characterOptions.length}</span>
                           </div>
                          <ResponsiveContainer width="100%" height="100%">
                            <PieChart>
                              <Pie
                                data={characterOptions.slice(0, 5)}
                                dataKey="tokens"
                                nameKey="name"
                                cx="50%"
                                cy="50%"
                                innerRadius={45}
                                outerRadius={60}
                                paddingAngle={4}
                                stroke="none"
                              >
                                {characterOptions.slice(0, 5).map((_, i) => (
                                  <Cell key={i} fill={COLORS[i % COLORS.length]} />
                                ))}
                              </Pie>
                            </PieChart>
                          </ResponsiveContainer>
                        </div>
                        <div className="flex-1 space-y-3">
                          {characterOptions.slice(0, 4).map((c, i) => (
                            <div key={c.id} className="flex items-center gap-3">
                              <div
                                className="h-2 w-2 rounded-full shrink-0"
                                style={{ backgroundColor: COLORS[i % COLORS.length] }}
                              />
                              <span className={cn(typography.bodySmall.size, "text-fg/70 truncate flex-1")}>{c.name}</span>
                              <span className={cn(typography.caption.size, "text-fg/40 font-bold")}>
                                {formatCompactNumber(c.tokens)}
                              </span>
                            </div>
                          ))}
                        </div>
                      </div>
                    </div>
                  )}
                </div>

                <div className={cn(
                  "overflow-hidden",
                  components.card.base,
                  "bg-fg/5 border-fg/10"
                )}>
                  <div className="flex items-center justify-between px-5 py-4 border-b border-fg/10 bg-fg/5">
                    <h3 className={cn(typography.h3.size, typography.h3.weight, "text-fg")}>{t("usageAnalytics.page.recentActivity")}</h3>
                    {filteredRecords.length > 5 && (
                      <div className="flex items-center gap-4">
                        <button
                          onClick={() => setShowAllActivity(true)}
                          className={cn(typography.caption.size, typography.caption.weight, "text-fg/40 hover:text-fg/60 transition-colors")}
                        >
                          {t("usageAnalytics.page.quickView")}
                        </button>
                        <button
                          onClick={() => navigate("/settings/usage/activity")}
                          className={cn(
                            "flex items-center gap-1 transition-all",
                            typography.caption.size, 
                            typography.caption.weight,
                            "text-accent hover:opacity-80"
                          )}
                        >
                          {t("usageAnalytics.page.viewAll")}
                          <ChevronRight className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    )}
                  </div>
                  <div className="divide-y divide-fg/10">
                    {filteredRecords.slice(0, 5).map((r) => (
                      <ActivityItem key={r.id} request={r} />
                    ))}
                    {filteredRecords.length === 0 && (
                      <div className="py-20 text-center">
                        <Calendar className="h-10 w-10 text-fg/10 mx-auto mb-4" />
                        <p className={cn(typography.body.size, "text-fg/40")}>{t("common.labels.none")}</p>
                        <p className={cn(typography.caption.size, "text-fg/30 mt-1.5")}>{t("usageAnalytics.page.startChatting")}</p>
                      </div>
                    )}
                  </div>
                </div>
              </motion.div>
            ) : (
              <motion.div
                key="appTime"
                {...animations.fadeIn}
                className="space-y-6"
              >
                <div className={cn(
                  "flex flex-wrap items-center gap-1.5 p-1.5 bg-fg/5 border border-fg/10",
                  radius.lg
                )}>
                  {[
                    { key: "today", label: t("usageAnalytics.page.today") },
                    { key: "week", label: t("usageAnalytics.page.last7Days") },
                    { key: "month", label: t("usageAnalytics.page.last30Days") },
                    { key: "all", label: t("usageAnalytics.page.all") },
                    { key: "custom", label: t("usageAnalytics.page.custom") },
                  ].map(({ key, label }) => (
                    <button
                      key={key}
                      onClick={() => {
                        if (key === "custom") {
                          setShowCustomDatePicker(true);
                          return;
                        }
                        setAppTimePreset(key as typeof appTimePreset);
                      }}
                      className={cn(
                        "px-4 py-1.5 rounded-lg transition-all duration-200",
                        typography.caption.size,
                        typography.caption.weight,
                        appTimePreset === key 
                          ? "bg-fg/10 text-fg shadow-sm" 
                          : "text-fg/40 hover:text-fg/70 hover:bg-fg/5"
                      )}
                    >
                      {label}
                    </button>
                  ))}
                </div>

                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
                  <StatCard
                    icon={Clock}
                    label={t("usageAnalytics.page.periodTotal")}
                    value={formatDurationMs(appTimeStats.rangeTotalMs)}
                    subValue={t("usageAnalytics.page.daysActive", { count: appTimeStats.selectedDays })}
                    trend={{
                      value: appTimeStats.rangeDeltaPct,
                      isUp: appTimeStats.rangeDeltaPct >= 0,
                    }}
                    highlight
                  />
                  <StatCard
                    icon={Activity}
                    label={t("usageAnalytics.page.dailyAvg")}
                    value={formatDurationMs(appTimeStats.dailyAvgInRangeMs)}
                    subValue={t("usageAnalytics.page.selectedPeriod")}
                  />
                  <StatCard
                    icon={Activity}
                    label={t("usageAnalytics.page.today")}
                    value={formatDurationMs(appTimeStats.todayMs)}
                    subValue={t("usageAnalytics.page.yesterdayValue", { value: formatDurationMs(appTimeStats.yesterdayMs) })}
                  />
                  <StatCard
                    icon={Activity}
                    label={t("usageAnalytics.page.thirtyDayAvg")}
                    value={formatDurationMs(appTimeStats.avg30Ms)}
                  />
                </div>

                <div className={cn(
                  "p-6",
                  components.card.base,
                  "bg-fg/5 border-fg/10"
                )}>
                  <div className="flex items-center justify-between mb-8">
                    <div>
                      <h3 className={cn(typography.h3.size, typography.h3.weight, "text-fg")}>{t("usageAnalytics.page.appTimeTrend")}</h3>
                      <p className={cn(typography.caption.size, "text-fg/40 mt-0.5")}>{t("usageAnalytics.page.usageDurationPerDay")}</p>
                    </div>
                    <div className={cn(typography.caption.size, "font-bold text-fg/50 px-3 py-1 bg-fg/5 rounded-full border border-fg/5")}>
                      {t("usageAnalytics.page.totalValue", { value: formatDurationMs(appTimeStats.rangeTotalMs) })}
                    </div>
                  </div>
                  <div className="h-64">
                    <ResponsiveContainer width="100%" height="100%">
                      <AreaChart
                        data={appTimeStats.byDayChart}
                        margin={{ top: 0, right: 0, left: -20, bottom: 0 }}
                      >
                        <defs>
                          <linearGradient id="appTimeGrad" x1="0" y1="0" x2="0" y2="1">
                            <stop offset="5%" stopColor="var(--color-accent)" stopOpacity={0.25} />
                            <stop offset="95%" stopColor="var(--color-accent)" stopOpacity={0} />
                          </linearGradient>
                        </defs>
                        <XAxis
                          dataKey="label"
                          tick={{ fill: "rgba(255,255,255,0.3)", fontSize: 10, fontWeight: 500 }}
                          axisLine={false}
                          tickLine={false}
                          dy={10}
                        />
                        <YAxis
                          tick={{ fill: "rgba(255,255,255,0.3)", fontSize: 10, fontWeight: 500 }}
                          axisLine={false}
                          tickLine={false}
                          tickFormatter={(v) => formatDurationMs(v)}
                        />
                        <Tooltip content={<AppTimeTooltip />} cursor={{ stroke: 'rgba(255,255,255,0.1)', strokeWidth: 1 }} />
                        <Area
                          type="monotone"
                          dataKey="ms"
                          name={t("usageAnalytics.page.activeTime")}
                          stroke="var(--color-accent)"
                          fill="url(#appTimeGrad)"
                          strokeWidth={2.5}
                          animationDuration={1500}
                        />
                      </AreaChart>
                    </ResponsiveContainer>
                  </div>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}
