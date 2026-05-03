import { useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, Loader2 } from "lucide-react";
import { useUsageTracking, RequestUsage } from "../../../core/usage";
import { ActivityItem, UsageRequestDetailSheet } from "./UsageActivityShared";
import { useI18n } from "../../../core/i18n/context";
import { typography, components, cn, animations } from "../../design-tokens";
import { motion, AnimatePresence } from "framer-motion";

const PAGE_SIZE = 50;

export function UsageActivityPage() {
  const { t } = useI18n();
  const { queryRecords } = useUsageTracking();
  const [records, setRecords] = useState<RequestUsage[]>([]);
  const [loading, setLoading] = useState(true);
  const [page, setPage] = useState(1);
  const [selectedRequest, setSelectedRequest] = useState<RequestUsage | null>(null);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      const allRecords = await queryRecords({});
      if (!cancelled) {
        setRecords(allRecords.sort((a, b) => b.timestamp - a.timestamp));
        setLoading(false);
      }
    };

    void load();

    return () => {
      cancelled = true;
    };
  }, [queryRecords]);

  const totalPages = Math.max(1, Math.ceil(records.length / PAGE_SIZE));

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  const pageRecords = useMemo(() => {
    const start = (page - 1) * PAGE_SIZE;
    return records.slice(start, start + PAGE_SIZE);
  }, [page, records]);

  return (
    <div className="min-h-screen bg-surface pb-32">
      <UsageRequestDetailSheet
        request={selectedRequest}
        isOpen={selectedRequest !== null}
        onClose={() => setSelectedRequest(null)}
      />

      <div className="mx-auto max-w-3xl px-4 py-6">
        {loading ? (
          <div className="flex flex-col items-center justify-center py-32 gap-4">
            <Loader2 className="h-10 w-10 animate-spin text-accent" />
            <p className={cn(typography.body.size, "text-fg/40")}>
              {t("usageAnalytics.activity.loading")}
            </p>
          </div>
        ) : (
          <motion.div 
            initial={animations.fadeIn.initial}
            animate={animations.fadeIn.animate}
            className="space-y-6"
          >
            <div className={cn(
              "flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between",
              components.card.base,
              "bg-fg/5 border-fg/10"
            )}>
              <div>
                <h2 className={cn(typography.h2.size, typography.h2.weight, "text-fg")}>
                  {t("usageAnalytics.activity.title")}
                </h2>
                <p className={cn(typography.bodySmall.size, "mt-0.5 text-fg/40")}>
                  {t("usageAnalytics.activity.recordsCount", { count: records.length.toLocaleString() })}
                </p>
              </div>
              
              {records.length > PAGE_SIZE && (
                <div className="flex items-center gap-3">
                  <div className={cn(typography.caption.size, "text-fg/40")}>
                    {t("usageAnalytics.activity.rangeOfTotal", {
                      start: (page - 1) * PAGE_SIZE + 1,
                      end: Math.min(page * PAGE_SIZE, records.length),
                      total: records.length,
                    })}
                  </div>
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      onClick={() => setPage((value) => Math.max(1, value - 1))}
                      disabled={page === 1}
                      className={cn(
                        "flex h-8 w-8 items-center justify-center rounded-lg border border-fg/10 bg-fg/5 text-fg transition-all",
                        "hover:bg-fg/10 disabled:cursor-not-allowed disabled:opacity-20"
                      )}
                    >
                      <ChevronLeft className="h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      onClick={() => setPage((value) => Math.min(totalPages, value + 1))}
                      disabled={page === totalPages}
                      className={cn(
                        "flex h-8 w-8 items-center justify-center rounded-lg border border-fg/10 bg-fg/5 text-fg transition-all",
                        "hover:bg-fg/10 disabled:cursor-not-allowed disabled:opacity-20"
                      )}
                    >
                      <ChevronRight className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              )}
            </div>

            <div className={cn(
              "overflow-hidden",
              components.card.base,
              "bg-fg/5 border-fg/10"
            )}>
              <div className="divide-y divide-fg/10">
                <AnimatePresence mode="popLayout">
                  {pageRecords.map((request) => (
                    <ActivityItem
                      key={request.id}
                      request={request}
                      onClick={setSelectedRequest}
                      showChevron
                    />
                  ))}
                </AnimatePresence>
                {pageRecords.length === 0 && (
                  <div className={cn(typography.body.size, "px-4 py-16 text-center text-fg/40")}>
                    {t("common.labels.none")}
                  </div>
                )}
              </div>
            </div>

            {records.length > PAGE_SIZE && (
              <div className="flex items-center justify-between">
                <button
                  type="button"
                  onClick={() => setPage((value) => Math.max(1, value - 1))}
                  disabled={page === 1}
                  className={cn(
                    "flex items-center gap-2 rounded-xl border border-fg/10 px-4 py-2 text-sm font-medium text-fg transition-all",
                    "hover:bg-fg/5 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-30"
                  )}
                >
                  <ChevronLeft className="h-4 w-4" />
                  {t("usageAnalytics.activity.previous")}
                </button>
                <div className={cn(typography.caption.size, "font-medium text-fg/40")}>
                  {t("usageAnalytics.activity.pageOf", { page, total: totalPages })}
                </div>
                <button
                  type="button"
                  onClick={() => setPage((value) => Math.min(totalPages, value + 1))}
                  disabled={page === totalPages}
                  className={cn(
                    "flex items-center gap-2 rounded-xl border border-fg/10 px-4 py-2 text-sm font-medium text-fg transition-all",
                    "hover:bg-fg/5 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-30"
                  )}
                >
                  {t("usageAnalytics.activity.next")}
                  <ChevronRight className="h-4 w-4" />
                </button>
              </div>
            )}
          </motion.div>
        )}
      </div>
    </div>
  );
}
