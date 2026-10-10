import { t } from "@xcss/web/admin-ui/i18n";
import { useEffect, useState } from "react";
import { errorRequestId } from "@xcss/web/admin-shell";
import { Button, EmptyState, ErrorState, LoadingState, Table } from "@xcss/web/admin-ui";
import { PageNavigation } from "./PageNavigation";
import { isAdminLogs, request, type AdminLogs, type BackupInstance } from "./api";
import { actionLabel } from "./display-labels";

import { DateRangeField, type CalendarDateRange } from "@xcss/web/admin-ui/date-range";
import "@xcss/web/admin-ui/date-range.css";

type Failure = { requestId?: string };
const LOG_RESPONSE_BYTES = 1024 * 1024;

export function LogsView({ refreshGeneration, instance }: { refreshGeneration: number; instance: BackupInstance | null }) {
  const [selectedRange, setSelectedRange] = useState<CalendarDateRange | null>(null);
  const [draftValid, setDraftValid] = useState(false);
  const [selection, setSelection] = useState<{range: CalendarDateRange | null; cursor: string | null; refreshGeneration: number}>({range: null, cursor: null, refreshGeneration});
  const requestedRange = selection.range;
  const cursor = selection.refreshGeneration === refreshGeneration ? selection.cursor : null;
  const instanceId = instance?.id ?? null;
  const [result, setResult] = useState<AdminLogs | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [generation, setGeneration] = useState(0);
  useEffect(() => {
    const controller = new AbortController(); setResult(null); setFailure(null);
    const query = new URLSearchParams();
    if (requestedRange !== null) {
      if (requestedRange.start === requestedRange.end) query.set("date", requestedRange.start);
      else { query.set("start_date", requestedRange.start); query.set("end_date", requestedRange.end); }
    }
    if (cursor !== null) query.set("cursor", cursor);
    if (instanceId !== null) query.set("instance_id", instanceId);
    const path = "/api/v1/admin/logs" + (query.size === 0 ? "" : `?${query}`);
    void request(path, (value): value is AdminLogs => isAdminLogs(value) && value.instance_id === instanceId && (requestedRange === null || value.date === requestedRange.start && (value.end_date ?? value.date) === requestedRange.end), { signal: controller.signal, maxResponseBytes: LOG_RESPONSE_BYTES, timeoutMs: 10_000 })
      .then(value => { if (!controller.signal.aborted) { setResult(value); if (requestedRange === null) setSelectedRange({ start: value.date, end: value.end_date ?? value.date }); } })
      .catch(error => { if (!controller.signal.aborted) setFailure({ requestId: errorRequestId(error) }); });
    return () => controller.abort();
  }, [requestedRange, cursor, instanceId, generation, refreshGeneration]);
  const logs = result !== null && selectedRange !== null && result.date === selectedRange?.start && (result.end_date ?? result.date) === selectedRange?.end && result.instance_id === instanceId ? result.logs : null;
  const loading = result === null && failure === null;
  const emptyMessage = logs?.length === 0 ? cursor === null ? t("所选日期范围暂无日志", "No logs in the selected date range") : t("本页暂无日志，请返回第一页。", "No logs on this page. Return to the first page.") : null;
  const move = (cursor: string | null) => { setSelection({range: selectedRange, cursor, refreshGeneration}); setResult(null); };
  const first = () => { move(null); setGeneration(value => value + 1); };
  return <section className="xcss-content-stack" aria-label={t("备份日志", "Backup logs")}>
    <div className="xcss-content-panel xcss-content-stack">
      <p>{t("实例：{0}", "Instance: {0}", [instance?.name ?? t("全部实例", "All instances")])}</p>
      <div className="xcss-log-date-controls">
        <label htmlFor="media-log-date-start-year">{t("日志日期范围（服务器时区）", "Log date range (server time zone)")}</label>
        <div className="xcss-actions">{instance && <Button onClick={() => { window.location.hash = "logs"; }}>{t("全部实例", "All instances")}</Button>}<Button onClick={first} disabled={loading || selectedRange === null || !draftValid}>{t("刷新日志", "Refresh logs")}</Button></div>
        <DateRangeField id="media-log-date" value={selectedRange} disabled={selectedRange === null && failure === null} onValidityChange={setDraftValid}
          onApply={range => { setSelectedRange(range); setSelection({range, cursor:null, refreshGeneration}); setGeneration(value => value + 1); setResult(null); setFailure(null); }} />
      </div>
    </div>
    {emptyMessage !== null ? <EmptyState>{emptyMessage}</EmptyState>
      : failure ? <ErrorState requestId={failure.requestId} onRetry={() => setGeneration(value => value + 1)}>{t("日志暂不可用，请重试。", "Logs are temporarily unavailable. Please retry.")}</ErrorState>
      : logs === null ? <LoadingState>{t("正在加载日志…", "Loading logs…")}</LoadingState>
      : <Table aria-label={t("备份日志记录", "Backup log records")}><thead><tr><th>{t("服务器时间", "Server time")}</th><th>{t("操作", "Action")}</th><th>{t("对象", "Entity")}</th></tr></thead><tbody>{logs.map(log => <tr key={log.sequence}><td><time>{log.occurred_at}</time></td><td>{actionLabel(log.action)} <code>{log.action}</code></td><td>{log.entity_id}</td></tr>)}</tbody></Table>}
    <PageNavigation page={logs === null ? null : result} loading={loading} firstLabel={t("首页", "First page")} canRestart={cursor !== null} first={first} move={move} label={t("日志分页", "Log pages")} />
  </section>;
}
