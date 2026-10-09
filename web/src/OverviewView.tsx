import { getLocale, t } from "@xcss/admin-ui/i18n";
import { useEffect, useState } from "react";
import { errorRequestId, useAdminApplication } from "@xcss/admin-shell";
import { Button, EmptyState, ErrorState, LoadingState, StatusBadge, Table } from "@xcss/admin-ui";
import { isOverview, isUndefined, request, type BackupInstance, type Overview } from "./api";
import { PageNavigation } from "./PageNavigation";
import { lastSeen, pairingLabel } from "./display-labels";

type Failure = { requestId?: string };
export function OverviewView({ refreshGeneration }: { refreshGeneration: number }) {
  const [selection, setSelection] = useState<{cursor: string | null; refreshGeneration: number}>({cursor: null, refreshGeneration});
  const cursor = selection.refreshGeneration === refreshGeneration ? selection.cursor : null;
  const [overview, setOverview] = useState<Overview | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [generation, setGeneration] = useState(0);
  const first = () => { setSelection({cursor:null, refreshGeneration}); setGeneration(value => value + 1); };
  const move = (cursor: string) => { setOverview(null); setSelection({cursor, refreshGeneration}); };
  useEffect(() => {
    const controller = new AbortController(); setOverview(null); setFailure(null);
    const path = "/api/v1/admin/overview" + (cursor === null ? "" : `?cursor=${encodeURIComponent(cursor)}`);
    void request(path, isOverview, {signal:controller.signal, maxResponseBytes:1024*1024, timeoutMs:10_000})
      .then(value => { if (!controller.signal.aborted) setOverview(value); })
      .catch(error => { if (!controller.signal.aborted) setFailure({requestId:errorRequestId(error)}); });
    return () => controller.abort();
  }, [cursor, refreshGeneration, generation]);
  const { notify } = useAdminApplication();
  const [deleteCandidate, setDeleteCandidate] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteFailure, setDeleteFailure] = useState<Failure | null>(null);
  async function remove(instance: BackupInstance) {
    if (deleting) return;
    setDeleting(true); setDeleteFailure(null);
    try {
      await request(`/api/v1/admin/instances/${instance.id}`, isUndefined, { method: "DELETE" });
      setDeleteCandidate(null); setDeleteFailure(null); first();
      notify(instance.status === "cancelled" || instance.status === "revoked" ? t("实例已删除", "Instance deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry"));
    } catch (error) { setDeleteFailure({ requestId: errorRequestId(error) }); }
    finally { setDeleting(false); }
  }
  if (failure) return <ErrorState requestId={failure.requestId} onRetry={first}>{t("备份数据暂不可用，请重试。", "Backup data is temporarily unavailable. Please retry.")}</ErrorState>;
  if (overview === null) return <LoadingState>{t("正在载入备份数据…", "Loading backup data…")}</LoadingState>;
  return <div className="media-sections"><section className="xcss-content-stack"><Table className="xcss-statistics-table" aria-label={t("实例统计", "Instance statistics")}>
    <thead><tr><th scope="col">{t("统计项", "Metric")}</th><th scope="col">{t("总数 / 在线", "Total / online")}</th></tr></thead>
    <tbody>
      <tr><th scope="row">{t("总数", "Total")}</th><td>{overview.total_users} / {overview.online_users}</td></tr>
      <tr><th scope="row">{t("媒体已用", "Media storage used")}</th><td>{bytes(overview.used_bytes)}</td></tr>
      <tr><th scope="row">{t("上传预留空间", "Reserved upload space")}</th><td>{bytes(overview.pending_bytes)}</td></tr>
      <tr><th scope="row">{t("已分配配额", "Allocated quota")}</th><td>{bytes(overview.quota_bytes)}{overview.unlimited_users > 0 ? t(" + 不限", " + Unlimited") : ""}</td></tr>
    </tbody></Table>
  </section><section className="xcss-content-stack" aria-label={t("实例", "Instances")}>
    {deleteFailure && <ErrorState requestId={deleteFailure.requestId}>{t("删除未能确认，请刷新实例列表核对。", "Deletion could not be confirmed. Refresh and check the instance list.")}</ErrorState>}
    {overview.users.length === 0 ? <EmptyState>{overview.total_users === 0 ? t("暂无备份实例", "No backup instances") : t("本页暂无实例，请刷新列表返回第一页。", "No instances on this page. Refresh the list to return to the first page.")}</EmptyState> : <Table aria-label={t("实例列表", "Instance list")}>
      <thead><tr><th>{t("实例", "Instance")}</th><th>{t("配对状态", "Pairing status")}</th><th>{t("在线状态", "Online status")}</th><th>{t("操作系统/架构", "Operating system / architecture")}</th><th>{t("最后在线", "Last seen")}</th><th>{t("已用容量 / 配额", "Used / quota")}</th><th>{t("删除", "Delete")}</th></tr></thead>
      <tbody>{overview.users.map(user => { const client = user.instances[0]; return <tr key={user.id}>
        <th scope="row"><a className="xcss-instance-link" href={"#details/" + encodeURIComponent(user.id)}>{user.display_name}</a></th><td><StatusBadge status={client ? pairingLabel(client.status) : "—"} /></td><td><StatusBadge status={client?.online ? t("在线", "Online") : t("离线", "Offline")} /></td><td>{client?.platform ?? "—"}</td><td>{lastSeen(client?.last_seen_at)}</td><td>{bytes(user.used_bytes)} / {user.quota_bytes === 0 ? t("不限", "Unlimited") : bytes(user.quota_bytes)}</td><td>{client ? <div className="xcss-actions">{deleteCandidate === client.id ? <><Button disabled={deleting} onClick={() => setDeleteCandidate(null)}>{t("取消", "Cancel")}</Button><Button className="xcss-danger" disabled={deleting} onClick={() => void remove(client)}>{deleting ? t("正在删除…", "Deleting…") : t("确认删除", "Confirm delete")}</Button></> : <Button disabled={deleting} onClick={() => { setDeleteFailure(null); setDeleteCandidate(client.id); }}>{t("删除", "Delete")}</Button>}</div> : "—"}</td>
      </tr>; })}</tbody>
    </Table>}
    <PageNavigation page={overview} canRestart={cursor !== null} first={first} move={move} label={t("实例分页", "Instance pages")} />
  </section></div>;
}


function bytes(value: number): string {
  for (const [scale, label] of [[2 ** 40, "TiB"], [2 ** 30, "GiB"], [2 ** 20, "MiB"], [2 ** 10, "KiB"]] as const) {
    if (value >= scale) return (value / scale).toLocaleString(getLocale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 }) + " " + label;
  }
  return value + " B";
}
