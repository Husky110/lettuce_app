import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUpDown, Copy, Cpu, HardDrive, RefreshCw, Search, Trash2 } from "lucide-react";

import { cn } from "../../design-tokens";
import { confirmBottomMenu } from "../../components/ConfirmBottomMenu";
import { toast } from "../../components/toast";
import { useI18n } from "../../../core/i18n/context";

type InstalledGgufModel = {
  modelId: string;
  filename: string;
  path: string;
  size: number;
  quantization: string;
  isMmproj: boolean;
  architecture?: string | null;
  contextLength?: number | null;
};

type SortField = "params" | "arch" | "context" | "size";
type SortDirection = "desc" | "asc";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / Math.pow(1024, index);
  return `${value.toFixed(index > 1 ? 1 : 0)} ${units[index]}`;
}

function extractParamSize(modelId: string, filename: string): string | null {
  const tagSource = `${modelId} ${filename}`;
  const direct = tagSource.match(/(?:^|[-_/ ])(\d+(?:\.\d+)?)(b|m|k|t)(?:[-_/ .]|$)/i);
  if (!direct) return null;
  return `${direct[1]}${direct[2].toUpperCase()}`;
}

function paramSizeToNumber(value: string | null): number | null {
  if (!value) return null;
  const match = value.match(/^(\d+(?:\.\d+)?)([KMBT])$/i);
  if (!match) return null;
  const numeric = Number(match[1]);
  const unit = match[2].toUpperCase();
  const multiplier =
    unit === "T"
      ? 1_000_000_000_000
      : unit === "B"
        ? 1_000_000_000
        : unit === "M"
          ? 1_000_000
          : 1_000;
  return numeric * multiplier;
}

function deriveDisplayName(filename: string): string {
  return (filename.split("/").pop() || filename).replace(/\.gguf$/i, "");
}

function formatContextLength(contextLength?: number | null): string {
  if (!contextLength || contextLength <= 0) return "—";
  return `${contextLength.toLocaleString()} ctx`;
}

function getModelTypeLabel(model: InstalledGgufModel): "MMPROJ" | "LLM" {
  return model.isMmproj ? "MMPROJ" : "LLM";
}

export function InstalledModelsPage() {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [modelsDir, setModelsDir] = useState("");
  const [models, setModels] = useState<InstalledGgufModel[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [deletingPath, setDeletingPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sortField, setSortField] = useState<SortField | null>(null);
  const [sortDirection, setSortDirection] = useState<SortDirection | null>(null);

  const loadModels = useCallback(async (mode: "initial" | "refresh" = "initial") => {
    if (mode === "initial") {
      setLoading(true);
    } else {
      setRefreshing(true);
    }

    try {
      const [dir, downloaded] = await Promise.all([
        invoke<string>("hf_get_gguf_models_dir"),
        invoke<InstalledGgufModel[]>("hf_list_downloaded_models"),
      ]);
      setModelsDir(dir);
      setModels(
        [...downloaded].sort((left, right) => {
          if (left.modelId !== right.modelId) {
            return left.modelId.localeCompare(right.modelId);
          }
          return left.filename.localeCompare(right.filename);
        }),
      );
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void loadModels("initial");
  }, [loadModels]);

  const filteredModels = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return models;
    return models.filter((model) => {
      return (
        model.modelId.toLowerCase().includes(needle) ||
        model.filename.toLowerCase().includes(needle) ||
        model.path.toLowerCase().includes(needle) ||
        (model.architecture ?? "").toLowerCase().includes(needle) ||
        model.quantization.toLowerCase().includes(needle)
      );
    });
  }, [models, query]);

  const sortedModels = useMemo(() => {
    if (!sortField || !sortDirection) return filteredModels;

    const compareNullableNumbers = (left: number | null, right: number | null) => {
      if (left === null && right === null) return 0;
      if (left === null) return 1;
      if (right === null) return -1;
      return left - right;
    };

    const compareNullableStrings = (left: string | null, right: string | null) => {
      if (!left && !right) return 0;
      if (!left) return 1;
      if (!right) return -1;
      return left.localeCompare(right);
    };

    const direction = sortDirection === "desc" ? -1 : 1;
    return [...filteredModels].sort((left, right) => {
      let comparison = 0;

      if (sortField === "params") {
        comparison = compareNullableNumbers(
          paramSizeToNumber(extractParamSize(left.modelId, left.filename)),
          paramSizeToNumber(extractParamSize(right.modelId, right.filename)),
        );
      } else if (sortField === "arch") {
        comparison = compareNullableStrings(left.architecture ?? null, right.architecture ?? null);
      } else if (sortField === "context") {
        comparison = compareNullableNumbers(
          left.contextLength ?? null,
          right.contextLength ?? null,
        );
      } else if (sortField === "size") {
        comparison = left.size - right.size;
      }

      if (comparison === 0) {
        return left.filename.localeCompare(right.filename);
      }

      return comparison * direction;
    });
  }, [filteredModels, sortDirection, sortField]);

  const totalSize = useMemo(
    () => sortedModels.reduce((sum, model) => sum + model.size, 0),
    [sortedModels],
  );

  const handleCopyPath = useCallback(async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      toast.success(t("installedModels.toasts.pathCopied"), path);
    } catch (err) {
      toast.error(
        t("installedModels.toasts.copyFailed"),
        err instanceof Error ? err.message : String(err),
      );
    }
  }, []);

  const handleDeleteModel = useCallback(
    async (model: InstalledGgufModel) => {
      const confirmed = await confirmBottomMenu({
        title: t("installedModels.confirm.deleteTitle"),
        message: t("installedModels.confirm.deleteMessage", { filename: model.filename }),
        confirmLabel: t("common.buttons.delete"),
        destructive: true,
      });
      if (!confirmed) return;

      try {
        setDeletingPath(model.path);
        await invoke("hf_delete_downloaded_model", { filePath: model.path });
        toast.success(t("installedModels.toasts.modelDeleted"), model.filename);
        await loadModels("refresh");
      } catch (err) {
        toast.error(
          t("installedModels.toasts.deleteFailed"),
          err instanceof Error ? err.message : String(err),
        );
      } finally {
        setDeletingPath(null);
      }
    },
    [loadModels],
  );

  const cycleSort = useCallback(
    (field: SortField) => {
      if (sortField !== field) {
        setSortField(field);
        setSortDirection("desc");
        return;
      }
      if (sortDirection === "desc") {
        setSortDirection("asc");
        return;
      }
      if (sortDirection === "asc") {
        setSortField(null);
        setSortDirection(null);
        return;
      }
      setSortDirection("desc");
    },
    [sortDirection, sortField],
  );

  const sortIndicator = useCallback(
    (field: SortField) => {
      if (sortField !== field || !sortDirection) return "";
      return sortDirection === "desc" ? " ↓" : " ↑";
    },
    [sortDirection, sortField],
  );

  const renderSortHeader = useCallback(
    (field: SortField, label: string) => {
      const isActive = sortField === field && !!sortDirection;
      return (
        <button
          onClick={() => cycleSort(field)}
          className={cn(
            "group flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left transition",
            isActive ? "bg-fg/[0.06] text-fg/80" : "text-fg/35 hover:text-fg/65 hover:bg-fg/[0.03]",
          )}
        >
          <span>{label}</span>
          <span
            className={cn(
              "inline-flex items-center text-[10px] transition",
              isActive ? "text-fg/75" : "text-fg/25 group-hover:text-fg/50",
            )}
          >
            {isActive ? (
              sortIndicator(field)
            ) : (
              <ArrowUpDown size={11} strokeWidth={2} className="translate-y-[0.5px]" />
            )}
          </span>
        </button>
      );
    },
    [cycleSort, sortDirection, sortField, sortIndicator],
  );

  return (
    <div className="mx-auto flex h-full w-full max-w-[1440px] flex-col gap-4 pb-6">
      <section className="space-y-3 rounded-xl border border-fg/10 bg-fg/[0.03] p-4">
        <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-sm font-medium text-fg">
              <HardDrive size={15} className="text-fg/60" />
              <span>{t("installedModels.title")}</span>
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2 text-sm text-fg/55">
              <button
                onClick={() => void handleCopyPath(modelsDir)}
                className="inline-flex max-w-full items-center gap-2 rounded-md border border-fg/10 px-2.5 py-1.5 text-left transition hover:bg-fg/[0.05] hover:text-fg"
              >
                <Copy size={12} />
                <span className="truncate font-mono text-[12px]">{modelsDir || "…"}</span>
              </button>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div className="rounded-md border border-fg/10 px-3 py-2 text-sm text-fg/70">
              {t("installedModels.fileCount", { count: sortedModels.length })}
            </div>
            <div className="rounded-md border border-fg/10 px-3 py-2 text-sm text-fg/70">
              {formatBytes(totalSize)}
            </div>
            <button
              onClick={() => void loadModels("refresh")}
              className={cn(
                "inline-flex items-center gap-2 rounded-md border border-fg/10 px-3 py-2 text-sm font-medium text-fg/80 transition hover:bg-fg/[0.06]",
              )}
            >
              <RefreshCw size={14} className={refreshing ? "animate-spin" : ""} />
              {t("common.buttons.refresh")}
            </button>
          </div>
        </div>

        <div className="flex items-center gap-3 rounded-md border border-fg/10 bg-bg px-3 py-2.5">
          <Search size={15} className="text-fg/35" />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("installedModels.searchPlaceholder")}
            className="w-full bg-transparent text-sm text-fg outline-none placeholder:text-fg/35"
          />
        </div>
      </section>

      {loading ? (
        <div className="rounded-xl border border-fg/10 bg-fg/[0.03] px-4 py-10 text-sm text-fg/55">
          {t("installedModels.loading")}
        </div>
      ) : error ? (
        <div className="rounded-xl border border-red-500/20 bg-red-500/5 px-4 py-4 text-sm text-red-200">
          {t("installedModels.loadFailed", { error })}
        </div>
      ) : sortedModels.length === 0 ? (
        <div className="rounded-xl border border-dashed border-fg/10 bg-fg/[0.02] px-4 py-12 text-center">
          <div className="mx-auto mb-3 flex h-11 w-11 items-center justify-center rounded-lg border border-fg/10 bg-fg/[0.03]">
            <Cpu size={18} className="text-fg/45" />
          </div>
          <div className="text-sm font-medium text-fg">{t("installedModels.empty.title")}</div>
          <p className="mt-1 text-sm text-fg/50">
            {t("installedModels.empty.description")}
          </p>
        </div>
      ) : (
        <div className="overflow-hidden rounded-xl border border-fg/10 bg-fg/[0.03]">
          <div className="hidden grid-cols-[minmax(0,1.45fr)_86px_110px_110px_130px_120px_168px] items-center gap-3 border-b border-fg/10 px-4 py-3 text-[11px] font-medium uppercase tracking-[0.16em] text-fg/35 lg:grid">
            <div>{t("common.labels.name")}</div>
            <div>{t("installedModels.columns.type")}</div>
            {renderSortHeader("params", t("installedModels.columns.params"))}
            {renderSortHeader("arch", t("installedModels.columns.arch"))}
            {renderSortHeader("context", t("installedModels.columns.context"))}
            {renderSortHeader("size", t("installedModels.columns.size"))}
            <div className="text-right">{t("installedModels.columns.action")}</div>
          </div>
          {sortedModels.map((model, index) => {
            const paramSize = extractParamSize(model.modelId, model.filename);
            return (
              <div
                key={`${model.path}-${model.filename}`}
                className={cn(
                  "px-4 py-4",
                  index !== sortedModels.length - 1 && "border-b border-fg/8",
                )}
              >
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1.45fr)_86px_110px_110px_130px_120px_168px] lg:items-center">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-semibold text-fg">
                      {deriveDisplayName(model.filename)}
                    </div>
                    <div className="truncate text-xs text-fg/45">{model.modelId}</div>
                    <div className="mt-2 flex flex-wrap items-center gap-1.5 lg:hidden">
                      <span
                        className={cn(
                          "rounded-md border px-2 py-0.5 text-[11px] font-medium",
                          model.isMmproj
                            ? "border-blue-400/20 bg-blue-400/10 text-blue-300"
                            : "border-emerald-400/20 bg-emerald-400/10 text-emerald-300",
                        )}
                      >
                        {getModelTypeLabel(model)}
                      </span>
                      <span className="rounded-md border border-fg/10 px-2 py-0.5 text-[11px] text-fg/60">
                        {model.quantization}
                      </span>
                      {paramSize ? (
                        <span className="rounded-md border border-fg/10 px-2 py-0.5 text-[11px] text-fg/60">
                          {paramSize}
                        </span>
                      ) : null}
                      <span className="rounded-md border border-fg/10 px-2 py-0.5 text-[11px] text-fg/60">
                        {model.architecture?.toUpperCase() || "—"}
                      </span>
                      <span className="rounded-md border border-fg/10 px-2 py-0.5 text-[11px] text-fg/60">
                        {formatContextLength(model.contextLength)}
                      </span>
                      <span className="rounded-md border border-fg/10 px-2 py-0.5 text-[11px] text-fg/60">
                        {formatBytes(model.size)}
                      </span>
                    </div>
                    <div className="mt-2 truncate font-mono text-[11px] text-fg/32">
                      {model.path}
                    </div>
                  </div>

                  <div className="hidden lg:block">
                    <span
                      className={cn(
                        "inline-flex rounded-md border px-2 py-1 text-[11px] font-medium",
                        model.isMmproj
                          ? "border-blue-400/20 bg-blue-400/10 text-blue-300"
                          : "border-emerald-400/20 bg-emerald-400/10 text-emerald-300",
                      )}
                    >
                      {getModelTypeLabel(model)}
                    </span>
                  </div>
                  <div className="hidden lg:block text-sm text-fg/70">{paramSize || "—"}</div>
                  <div className="hidden lg:block text-sm text-fg/70">
                    {model.architecture?.toUpperCase() || "—"}
                  </div>
                  <div className="hidden lg:block text-sm text-fg/70">
                    {formatContextLength(model.contextLength)}
                  </div>
                  <div className="hidden lg:block text-sm font-medium text-fg">
                    {formatBytes(model.size)}
                  </div>

                  <div className="flex items-center justify-end gap-3">
                    <span className="hidden rounded-md border border-fg/10 px-2 py-1 text-[11px] text-fg/55 lg:inline-flex">
                      {model.quantization}
                    </span>
                    <button
                      onClick={() => void handleCopyPath(model.path)}
                      className="inline-flex items-center gap-1 rounded-md border border-fg/10 bg-fg/[0.02] px-2.5 py-1.5 text-xs text-fg/65 transition hover:bg-fg/[0.06] hover:text-fg"
                    >
                      <Copy size={12} />
                      {t("common.buttons.copy")}
                    </button>
                    <button
                      onClick={() => void handleDeleteModel(model)}
                      disabled={deletingPath === model.path}
                      className={cn(
                        "inline-flex items-center gap-1 rounded-md border px-2.5 py-1.5 text-xs transition",
                        deletingPath === model.path
                          ? "cursor-not-allowed border-red-500/10 text-red-200/45"
                          : "border-red-500/20 bg-red-500/[0.08] text-red-200/85 hover:bg-red-500/[0.14]",
                      )}
                    >
                      <Trash2 size={12} />
                      {deletingPath === model.path
                        ? t("common.buttons.deleting")
                        : t("common.buttons.delete")}
                    </button>
                  </div>
                </div>
                <div className="mt-3 h-px bg-fg/0 lg:hidden" />
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
