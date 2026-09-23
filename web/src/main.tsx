import { t } from "@sarmg/admin-ui/i18n";
import { StrictMode, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { createSarmgAdminApplication, errorRequestId, useAdminApplication, InstancePageNavigation, InstanceHeaderActions, InstanceNameField } from "@sarmg/admin-shell";
import { Button, ConfirmDangerDialog, Dialog, EmptyState, ErrorState, FormField, LoadingState, StatusBadge, Table, TextField } from "@sarmg/admin-ui";
import "@sarmg/design-tokens/tokens.css";
import "@sarmg/design-tokens/tokens.dark.css";
import "@sarmg/design-tokens/reset.css";
import "@sarmg/design-tokens/accessibility.css";
import "../fonts/fonts.css";
import "@sarmg/admin-ui/styles.css";
import "./styles.css";
import { administratorApi, isBackupInstance, isBackupUser, isOverview, isUndefined, request, type BackupInstance, type BackupUser, type Overview } from "./api";

type Failure = { requestId?: string };
type View = "instances" | "details" | "logs";
const GIB = 1_073_741_824;
function currentView(): View {
  const hash = window.location.hash.slice(1);
  return hash.startsWith("details/") ? "details" : hash === "logs" ? "logs" : "instances";
}
function selectedUserFromLocation(): string | null {
  if (!window.location.hash.startsWith("#details/")) return null;
  try { return decodeURIComponent(window.location.hash.slice(9)); } catch { return null; }
}
function quotaBytes(value: FormDataEntryValue | null): number {
  const text = String(value ?? "").trim();
  const gib = Number(text);
  const bytes = Math.round(gib * GIB);
  if (text === "" || !Number.isFinite(gib) || gib < 0 || (gib > 0 && bytes === 0) || !Number.isSafeInteger(bytes)) {
    throw new Error("Invalid quota");
  }
  return bytes;
}
function Application() {
  const [view, setView] = useState<View>(currentView);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [generation, setGeneration] = useState(0);
  const [selected, setSelected] = useState<string | null>(selectedUserFromLocation);
  const [creating, setCreating] = useState(false);
  const [createFailure, setCreateFailure] = useState<Failure | null>(null);
  const reload = () => setGeneration(value => value + 1);
  async function createInstance() {
    if (creating) return;
    setCreating(true); setCreateFailure(null); window.location.hash = "instances";
    try {
      await request("/api/v2/admin/instances", isBackupInstance, { method: "POST", body: JSON.stringify({ name: t("新实例", "New instance") }) });
      reload(); notify(t("备份实例已创建，可在详细信息中查看授权码", "Backup instance created. Its authorization code is available in details"));
    } catch (error) { setCreateFailure({ requestId: errorRequestId(error) }); }
    finally { setCreating(false); }
  }
  const { notify } = useAdminApplication();
  useEffect(() => {
    const changed = () => { setView(currentView()); setSelected(selectedUserFromLocation()); }; window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  useEffect(() => {
    const controller = new AbortController(); setOverview(null); setFailure(null);
    void request("/api/v2/admin/overview", isOverview, { signal: controller.signal })
      .then(value => { if (!controller.signal.aborted) setOverview(value); })
      .catch(error => { if (!controller.signal.aborted) setFailure({ requestId: errorRequestId(error) }); });
    return () => controller.abort();
  }, [generation, view]);
  const user = overview?.users.find(item => item.id === selected);
  return <div className="media-business sarmg-content-stack">
    <InstanceHeaderActions create={() => void createInstance()} refresh={reload} refreshing={creating} />
    <InstancePageNavigation page={view} detailsDisabled={!selected} navigate={value => { window.location.hash = value === "details" && selected ? `details/${encodeURIComponent(selected)}` : value; }} />
    <h1 className="sarmg-visually-hidden">{view === "instances" ? t("备份实例列表", "Backup instance list") : view === "details" ? t("备份详细信息", "Backup details") : t("备份日志", "Backup logs")}</h1>
    {createFailure && <ErrorState requestId={createFailure.requestId}>{t("实例未能创建，请刷新列表核对后重试。", "The instance could not be created. Refresh the list before retrying.")}</ErrorState>}
    {failure ? <ErrorState requestId={failure.requestId} onRetry={reload}>{t("备份数据暂不可用，请重试。", "Backup data is temporarily unavailable. Please retry.")}</ErrorState>
      : overview === null ? <LoadingState>{t("正在载入备份数据…", "Loading backup data…")}</LoadingState>
      : view === "instances" ? <OverviewView overview={overview} reload={reload} /> : view === "logs" ? <LogsView /> : user ? <UsersView overview={{...overview, users:[user]}} reload={reload} /> : <EmptyState>{t("请选择一个备份实例。", "Select a backup instance.")}</EmptyState>}
  </div>;
}

function OverviewView({ overview, reload }: { overview: Overview; reload(): void }) {
  const { notify } = useAdminApplication();
  const [deleteCandidate, setDeleteCandidate] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteFailure, setDeleteFailure] = useState<Failure | null>(null);
  async function remove(instance: BackupInstance) {
    if (deleting) return;
    setDeleting(true); setDeleteFailure(null);
    try {
      await request(`/api/v2/admin/instances/${instance.id}`, isUndefined, { method: "DELETE" });
      setDeleteCandidate(null); setDeleteFailure(null); reload();
      notify(instance.status === "cancelled" || instance.status === "revoked" ? t("实例已删除", "Instance deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry"));
    } catch (error) { setDeleteFailure({ requestId: errorRequestId(error) }); }
    finally { setDeleting(false); }
  }
  const online = overview.users.filter(user => user.instances[0]?.online === true).length;
  return <div className="media-sections"><Section title={t("统计", "Statistics")}><Table aria-label={t("实例统计", "Instance statistics")}>
    <thead><tr><th scope="col">{t("统计项", "Metric")}</th><th scope="col">{t("总数 / 在线", "Total / online")}</th></tr></thead>
    <tbody>
      <tr><th scope="row">{t("总数", "Total")}</th><td>{overview.total_users} / {online}</td></tr>
      <tr><th scope="row">{t("媒体已用", "Media storage used")}</th><td>{bytes(overview.used_bytes)}</td></tr>
      <tr><th scope="row">{t("上传预留空间", "Reserved upload space")}</th><td>{bytes(overview.pending_bytes)}</td></tr>
      <tr><th scope="row">{t("已分配配额", "Allocated quota")}</th><td>{bytes(overview.quota_bytes)}{overview.unlimited_users > 0 ? t(" + 不限", " + Unlimited") : ""}</td></tr>
    </tbody></Table>
  </Section><Section title={t("实例列表", "Instance list")}>
    {deleteFailure && <ErrorState requestId={deleteFailure.requestId}>{t("删除未能确认，请刷新实例列表核对。", "Deletion could not be confirmed. Refresh and check the instance list.")}</ErrorState>}
    {overview.users.length === 0 ? <EmptyState>{t("暂无备份实例", "No backup instances")}</EmptyState> : <Table aria-label={t("实例列表", "Instance list")}>
      <thead><tr><th>{t("实例", "Instance")}</th><th>{t("配对状态", "Pairing status")}</th><th>{t("在线状态", "Online status")}</th><th>{t("操作系统/架构", "Operating system / architecture")}</th><th>{t("最后在线", "Last seen")}</th><th>{t("已用容量 / 配额", "Used / quota")}</th><th>{t("删除", "Delete")}</th></tr></thead>
      <tbody>{overview.users.map(user => { const client = user.instances[0]; return <tr key={user.id}>
        <th scope="row"><a className="sarmg-instance-link" href={"#details/" + encodeURIComponent(user.id)}>{user.display_name}</a></th><td><StatusBadge status={client?.status ?? "—"} /></td><td><StatusBadge status={client?.online ? t("在线", "Online") : t("离线", "Offline")} /></td><td>{client?.platform ?? "—"}</td><td>{client?.last_seen_at || t("尚未配对", "Not paired yet")}</td><td>{bytes(user.used_bytes)} / {user.quota_bytes === 0 ? t("不限", "Unlimited") : bytes(user.quota_bytes)}</td><td>{client ? <div className="sarmg-actions">{deleteCandidate === client.id ? <><Button disabled={deleting} onClick={() => setDeleteCandidate(null)}>{t("取消", "Cancel")}</Button><Button className="sarmg-danger" disabled={deleting} onClick={() => void remove(client)}>{deleting ? t("正在删除…", "Deleting…") : t("确认删除", "Confirm delete")}</Button></> : <Button disabled={deleting} onClick={() => { setDeleteFailure(null); setDeleteCandidate(client.id); }}>{t("删除", "Delete")}</Button>}</div> : "—"}</td>
      </tr>; })}</tbody>
    </Table>}
  </Section></div>;
}

function UsersView({ overview, reload }: { overview: Overview; reload(): void }) {
  return <div className="sarmg-content-stack">
    {overview.users.length === 0 ? <EmptyState>{t("暂无备份实例", "No backup instances yet")}</EmptyState> : overview.users.map(user => <div key={user.id} className="sarmg-content-stack"><PairingAccount user={user} /><BackupStatus user={user} /><BackupUserForm user={user} reload={reload} /><InstanceManager user={user} reload={reload} /></div>)}
  </div>;
}

function PairingAccount({ user }: { user: BackupUser }) {
  const instance = user.instances[0];
  if (!instance) return null;
  return <section className="sarmg-content-panel" aria-label={t("配对账户信息", "Pairing account information")}><h2>{user.display_name}</h2>
    <dl className="media-detail-list">
      <dt>{t("实例名称", "Instance name")}</dt><dd>{user.display_name}</dd>
      <dt>{t("实例 ID", "Instance ID")}</dt><dd><code>{instance.id}</code></dd>
      <dt>{t("实例授权码", "Instance authorization code")}</dt><dd><code>{instance.authorization_code}</code></dd>
      <dt>{t("配对状态", "Pairing status")}</dt><dd><StatusBadge status={instance.status} /></dd>
    </dl>
  </section>;
}

function BackupUserForm({ user, reload }: { user: BackupUser; reload(): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  async function save(input: Record<string, unknown>) {
    if (busy.current) return;
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request("/api/v2/admin/users/" + user.id, isBackupUser,
        { method: "PUT", body: JSON.stringify(input) });
      notify(t("备份实例已保存", "Backup instance saved")); reload();
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
    finally { busy.current = false; setPending(false); }
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (busy.current) return;
    const form = event.currentTarget, data = new FormData(form); setFailure(null);
    try {
      const quotaText = String(data.get("quota_gib") ?? "").trim();
      const input = { username: String(data.get("username") ?? ""), display_name: String(data.get("display_name") ?? ""),
        storage_path: String(data.get("storage_path") ?? ""),
        quota_bytes: quotaText === String(user.quota_bytes / GIB) ? user.quota_bytes : quotaBytes(quotaText) };
      void save(input);
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
  }
  const error = failure && <ErrorState requestId={failure.requestId}>{t("未能保存备份实例，请检查名称、路径和配额后重试。", "Unable to save the backup instance. Check the name, path, and quota, then retry.")}</ErrorState>;
  return <section className="media-card sarmg-content-panel" aria-label={t("实例设置", "Instance settings")}>
    <h2>{t("实例设置", "Instance settings")}</h2>
    <form aria-label={t("编辑备份实例 ", "Edit backup instance ") + user.display_name} aria-busy={pending} onSubmit={submit}>
      {error}
      <FormField label={t("实例名称", "Instance name")}><InstanceNameField name="display_name" defaultValue={user.display_name} required readOnly={pending} /></FormField>
      <input type="hidden" name="username" value={user.username} />
      <FormField label={t("存储路径", "Storage path")}><TextField name="storage_path" defaultValue={user.storage_path} required readOnly={pending} /></FormField>
      <FormField label={t("配额（GiB，0 表示不限）", "Quota (GiB; 0 means unlimited)")}><TextField name="quota_gib" type="number" min={0} step="any" defaultValue={user.quota_bytes / GIB} required readOnly={pending} /></FormField>
      <div className="sarmg-actions"><Button type="submit" disabled={pending}>{pending ? t("正在保存…", "Saving…") : t("保存设置", "Save settings")}</Button></div>
    </form>
  </section>;
}

function BackupStatus({ user }: { user: BackupUser }) {
  return <section className="sarmg-content-panel sarmg-content-stack" aria-label={t("备份状态", "Backup status")}>
    <h2>{t("备份状态", "Backup status")}</h2>
    {user.instances.map(instance => <div key={instance.id}><dl className="media-detail-list"><dt>{t("客户端", "Client")}</dt><dd>{instance.name}</dd><dt>{t("平台", "Platform")}</dt><dd>{instance.platform}</dd><dt>{t("在线状态", "Online status")}</dt><dd>{instance.online ? t("在线", "Online") : t("离线", "Offline")}</dd><dt>{t("最后在线", "Last seen")}</dt><dd>{instance.last_seen_at || t("尚未配对", "Not paired yet")}</dd></dl></div>)}
  </section>;
}

function InstanceManager({ user, reload }: { user: BackupUser; reload(): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [remove, setRemove] = useState<BackupInstance | null>(null);
  const [rotating, setRotating] = useState<BackupInstance | null>(null);
  const [failure, setFailure] = useState<{ action: "rotate" | "remove"; requestId?: string } | null>(null);
  async function rotate(instance: BackupInstance) {
    if (busy.current) return;
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request(`/api/v2/admin/instances/${instance.id}/authorization`, isBackupInstance, { method: "PUT" });
      setRotating(null);
      notify(t("授权码已更换，客户端必须重新配对", "Authorization code changed; the client must pair again")); reload();
    } catch (error) {
      setFailure({ action: "rotate", requestId: errorRequestId(error) });
    } finally { busy.current = false; setPending(false); }
  }
  async function removeInstance(instance: BackupInstance) {
    if (busy.current) return;
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request(`/api/v2/admin/instances/${instance.id}`, isUndefined, { method: "DELETE" });
      notify(instance.status === "cancelled" || instance.status === "revoked" ? t("实例信息已删除", "Instance entry deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry")); setRemove(null); reload();
    } catch (error) {
      setFailure({ action: "remove", requestId: errorRequestId(error) });
    } finally { busy.current = false; setPending(false); }
  }
  return <section className="sarmg-content-panel sarmg-content-stack" aria-label={t("实例操作", "Instance actions")}>
    <h2>{t("实例操作", "Instance actions")}</h2>
    <p>{t("实例拥有一个长期客户端授权码。服务端加密保存并可查看；更换后旧客户端立即失效并需要重新配对。", "The instance owns one long-lived client authorization code. The server stores it encrypted and keeps it viewable; changing it invalidates the old client and requires pairing again.")}</p>
    {user.instances.length > 0 && user.instances.map(instance => { const terminal = instance.status === "cancelled" || instance.status === "revoked"; return <div className="sarmg-content-stack" key={instance.id}>{user.instances.length > 1 && <h3>{instance.name}</h3>}<div className="sarmg-actions">{!terminal && <Button disabled={pending} onClick={() => { setFailure(null); setRotating(instance); }}>{t("更换授权码", "Change authorization code")}</Button>}<Button disabled={pending} onClick={() => { setFailure(null); setRemove(instance); }}>{instance.status === "pending" ? t("取消配对", "Cancel pairing") : terminal ? t("删除实例", "Delete instance") : t("撤销实例", "Revoke instance")}</Button></div></div>; })}
    {rotating && <ConfirmDangerDialog title={t("更换授权码", "Change authorization code")} description={t("更换授权码会立即撤销当前客户端凭据。客户端必须使用新授权码重新配对。", "Changing the authorization code immediately revokes the current client credential. The client must pair again using the new authorization code.")} pending={pending} onClose={() => { if (!busy.current) { setRotating(null); setFailure(null); } }} onConfirm={() => void rotate(rotating)}>{failure?.action === "rotate" && <ErrorState requestId={failure.requestId}>{t("更换结果未能确认，请关闭此窗口并刷新实例信息，核对授权码后再操作。", "The change could not be confirmed. Close this dialog and refresh the instance details to check the authorization code before continuing.")}</ErrorState>}</ConfirmDangerDialog>}
    {remove && (() => { const terminal = remove.status === "cancelled" || remove.status === "revoked"; return <ConfirmDangerDialog title={terminal ? t("删除实例", "Delete instance") : t("取消或撤销实例", "Cancel or revoke instance")} description={terminal ? t("永久删除这条终态且没有备份数据的实例信息。", "Permanently delete this terminal instance entry when it owns no backup data.") : t("授权码和访问令牌将立即失效。终态实例之后可从列表永久删除。", "The authorization code and access token are invalidated immediately. The terminal instance can then be permanently deleted from the list.")} pending={pending} onClose={() => { if (!pending) { setRemove(null); setFailure(null); } }} onConfirm={() => void removeInstance(remove)}>{failure?.action === "remove" && <ErrorState requestId={failure.requestId}>{terminal ? t("实例仍有备份记录或暂时无法删除，请处理后重试。", "The instance still owns backup records or cannot currently be deleted. Resolve the issue and retry.") : t("未能取消或撤销实例，请重试。", "Unable to cancel or revoke the instance. Please retry.")}</ErrorState>}</ConfirmDangerDialog>; })()}
  </section>;
}

function LogsView() {
  const [logs, setLogs] = useState<Array<{ sequence: number; action: string; entity_id: string; occurred_at: string }> | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [generation, setGeneration] = useState(0);
  useEffect(() => {
    const controller = new AbortController(); setLogs(null); setFailure(null);
    void request("/api/v2/admin/logs", (value): value is Array<{ sequence: number; action: string; entity_id: string; occurred_at: string }> => Array.isArray(value) && value.every(item => typeof item === "object" && item !== null && typeof (item as any).sequence === "number" && typeof (item as any).action === "string" && typeof (item as any).entity_id === "string" && typeof (item as any).occurred_at === "string"), { signal: controller.signal })
      .then(value => { if (!controller.signal.aborted) setLogs(value); })
      .catch(error => { if (!controller.signal.aborted) setFailure({ requestId: errorRequestId(error) }); });
    return () => controller.abort();
  }, [generation]);
  if (failure) return <ErrorState requestId={failure.requestId} onRetry={() => setGeneration(value => value + 1)}>{t("日志暂不可用，请重试。", "Logs are temporarily unavailable. Please retry.")}</ErrorState>;
  if (logs === null) return <LoadingState>{t("正在加载日志…", "Loading logs…")}</LoadingState>;
  return <section className="sarmg-content-stack"><h2>{t("日志", "Logs")}</h2>{logs.length === 0 ? <EmptyState>{t("暂无日志", "No logs")}</EmptyState> : <Table><thead><tr><th>{t("时间", "Time")}</th><th>{t("操作", "Action")}</th><th>{t("对象", "Entity")}</th></tr></thead><tbody>{logs.map(log => <tr key={log.sequence}><td>{log.occurred_at}</td><td>{log.action}</td><td>{log.entity_id}</td></tr>)}</tbody></Table>}</section>;
}

function Section({ title, children }: { title: string; children: ReactNode }) { return <section className="sarmg-content-stack"><h2>{title}</h2>{children}</section>; }
function bytes(value: number): string {
  for (const [scale, label] of [[2 ** 40, "TiB"], [2 ** 30, "GiB"], [2 ** 20, "MiB"], [2 ** 10, "KiB"]] as const) {
    if (value >= scale) return (value / scale).toFixed(1) + " " + label;
  }
  return value + " B";
}
const Root = createSarmgAdminApplication({ product: { name: "Media Backup" }, client: administratorApi,
  navigation: [], loginLandingHref: "#instances", routes: <Application /> });
const root = document.getElementById("root");
if (root === null) throw new Error("缺少 React 根节点");
createRoot(root).render(<StrictMode><Root /></StrictMode>);
