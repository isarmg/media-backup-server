import { t } from "@sarmg/admin-ui/i18n";
import { StrictMode, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { createSarmgAdminApplication, errorRequestId, useAdminApplication, InstancePageNavigation, InstanceHeaderActions, InstanceNameField } from "@sarmg/admin-shell";
import { Button, Checkbox, ConfirmDangerDialog, Dialog, EmptyState, ErrorState, FormField, LoadingState, StatusBadge, Table, TextField } from "@sarmg/admin-ui";
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
  const text = String(value ?? "").trim(), result = Number(text) * GIB;
  if (text === "" || !Number.isSafeInteger(result) || result < 0) throw new Error("Invalid quota");
  return result;
}
function Application() {
  const [view, setView] = useState<View>(currentView);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [generation, setGeneration] = useState(0);
  const [selected, setSelected] = useState<string | null>(selectedUserFromLocation);
  const [creating, setCreating] = useState(false);
  const reload = () => setGeneration(value => value + 1);
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
    <InstanceHeaderActions create={() => setCreating(true)} refresh={reload} />
    <InstancePageNavigation page={view} detailsDisabled={!selected} navigate={value => { window.location.hash = value === "details" && selected ? `details/${encodeURIComponent(selected)}` : value; }} />
    <h1 className="sarmg-visually-hidden">{view === "instances" ? t("备份实例列表", "Backup instance list") : view === "details" ? t("备份详细信息", "Backup details") : t("备份日志", "Backup logs")}</h1>
    {failure ? <ErrorState requestId={failure.requestId} onRetry={reload}>{t("备份数据暂不可用，请重试。", "Backup data is temporarily unavailable. Please retry.")}</ErrorState>
      : overview === null ? <LoadingState>{t("正在载入备份数据…", "Loading backup data…")}</LoadingState>
      : view === "instances" ? <OverviewView overview={overview} reload={reload} /> : view === "logs" ? <LogsView /> : <><a href="#instances">{t("返回实例列表", "Back to instance list")}</a>{user ? <UsersView overview={{...overview, users:[user]}} reload={reload} /> : <EmptyState>{t("请选择一个备份实例。", "Select a backup instance.")}</EmptyState>}</>}
    {creating && <CreateBackupInstanceDialog close={() => setCreating(false)} reload={reload} />}
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
      <thead><tr><th>{t("实例", "Instance")}</th><th>{t("备份状态", "Backup status")}</th><th>{t("配对状态", "Pairing status")}</th><th>{t("在线状态", "Online status")}</th><th>{t("客户端 / 平台", "Client / platform")}</th><th>{t("最后在线", "Last seen")}</th><th>{t("已用容量 / 配额", "Used / quota")}</th><th>{t("删除", "Delete")}</th></tr></thead>
      <tbody>{overview.users.map(user => { const client = user.instances[0]; return <tr key={user.id}>
        <th scope="row"><a href={"#details/" + encodeURIComponent(user.id)}>{user.display_name}</a></th><td><StatusBadge status={user.enabled ? t("已启用", "Enabled") : t("已停用", "Disabled")} /></td><td><StatusBadge status={client?.status ?? t("旧数据待补全", "Legacy entry incomplete")} /></td><td><StatusBadge status={client?.online ? t("在线", "Online") : t("离线", "Offline")} /></td><td>{client ? `${client.name} / ${client.platform}` : "—"}</td><td>{client?.last_seen_at || t("尚未配对", "Not paired yet")}</td><td>{bytes(user.used_bytes)} / {user.quota_bytes === 0 ? t("不限", "Unlimited") : bytes(user.quota_bytes)}</td><td>{client ? <div className="sarmg-actions">{deleteCandidate === client.id ? <><Button disabled={deleting} onClick={() => setDeleteCandidate(null)}>{t("取消", "Cancel")}</Button><Button className="sarmg-danger" disabled={deleting} onClick={() => void remove(client)}>{deleting ? t("正在删除…", "Deleting…") : t("确认删除", "Confirm delete")}</Button></> : <Button disabled={deleting} onClick={() => { setDeleteFailure(null); setDeleteCandidate(client.id); }}>{t("删除", "Delete")}</Button>}</div> : "—"}</td>
      </tr>; })}</tbody>
    </Table>}
  </Section></div>;
}

function UsersView({ overview, reload }: { overview: Overview; reload(): void }) {
  return <div className="media-sections"><p>{t("每个备份实例直接对应一个客户端授权码和独立存储配额；配额为 0 表示不限。", "Each backup instance directly owns one client authorization code and an isolated storage quota. A quota of 0 means unlimited.")}</p>
    <Section title={t("管理备份实例", "Manage backup instance")}><div className="media-grid">
      {overview.users.length === 0 ? <EmptyState>{t("暂无备份实例", "No backup instances yet")}</EmptyState> : overview.users.map(user => <div key={user.id} className="sarmg-content-stack"><BackupUserForm user={user} reload={reload} /><InstanceManager user={user} reload={reload} /></div>)}
    </div></Section></div>;
}

function BackupUserForm({ user, reload }: { user: BackupUser; reload(): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [disableInput, setDisableInput] = useState<Record<string, unknown> | null>(null);
  async function save(input: Record<string, unknown>, form?: HTMLFormElement) {
    if (busy.current) return;
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request("/api/v2/admin/users/" + user.id, isBackupUser,
        { method: "PUT", body: JSON.stringify(input) });
      form?.reset(); setDisableInput(null); notify(t("备份实例已保存", "Backup instance saved")); reload();
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
    finally { busy.current = false; setPending(false); }
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (busy.current) return;
    const form = event.currentTarget, data = new FormData(form); setFailure(null);
    try {
      const input = { username: String(data.get("username") ?? ""), display_name: String(data.get("display_name") ?? ""),
        storage_path: String(data.get("storage_path") ?? ""), quota_bytes: quotaBytes(data.get("quota_gib")),
        enabled: data.has("enabled") };
      if (user.enabled && !input.enabled) setDisableInput(input); else void save(input, form);
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
  }
  const error = failure && <ErrorState requestId={failure.requestId}>{t("未能保存备份实例，请检查名称、路径和配额后重试。", "Unable to save the backup instance. Check the name, path, and quota, then retry.")}</ErrorState>;
  return <article className="media-card sarmg-content-panel">
    <h3>{user.display_name}</h3><StatusBadge status={user.enabled ? t("已启用", "Enabled") : t("已停用", "Disabled")} />
    <form aria-label={t("编辑备份实例 ", "Edit backup instance ") + user.display_name} aria-busy={pending} onSubmit={submit}>
      {!disableInput && error}
      <FormField label={t("实例名称", "Instance name")}><InstanceNameField name="display_name" defaultValue={user.display_name} required readOnly={pending} /></FormField>
      <input type="hidden" name="username" value={user.username} />
      <FormField label={t("存储路径", "Storage path")}><TextField name="storage_path" defaultValue={user.storage_path} required readOnly={pending} /></FormField>
      <FormField label={t("配额（GiB，0 表示不限）", "Quota (GiB; 0 means unlimited)")}><TextField name="quota_gib" type="number" min={0} step="any" defaultValue={user.quota_bytes / GIB} required readOnly={pending} /></FormField>
      <label className="media-check"><Checkbox name="enabled" defaultChecked={user.enabled} disabled={pending} />{t("启用备份实例", "Enable backup instance")}</label>
      <div className="sarmg-actions"><Button type="submit" disabled={pending}>{pending ? t("正在保存…", "Saving…") : t("保存实例", "Save instance")}</Button></div>
    </form>
    {disableInput && <ConfirmDangerDialog title={t("停用备份实例 ", "Disable backup instance ") + user.display_name + "？"} description={t("该实例将无法继续上传，已有备份数据不会删除。", "This instance will no longer be able to upload. Existing backups will not be deleted.")} pending={pending}
      onClose={() => { if (!busy.current) { setDisableInput(null); setFailure(null); } }} onConfirm={() => void save(disableInput)}>{error}</ConfirmDangerDialog>}
  </article>;
}

function CreateBackupInstanceDialog({ close, reload }: { close(): void; reload(): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy.current) return;
    const data = new FormData(event.currentTarget);
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request("/api/v2/admin/instances", isBackupInstance, { method: "POST", body: JSON.stringify({
        name: String(data.get("name") ?? "").trim(),
      }) });
      reload(); close(); notify(t("备份实例已创建，可在详细信息中查看授权码", "Backup instance created. Its authorization code is available in details"));
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
    finally { busy.current = false; setPending(false); }
  }
  return <Dialog title={t("新建备份实例", "Create backup instance")} description={t("填写名称后会直接生成实例和长期授权码；存储路径自动分配，配额可在详情中调整。", "Enter a name to create the instance and its long-lived authorization code. Storage is allocated automatically, and quota can be changed in details.")} onClose={() => { if (!busy.current) close(); }}>
    <form aria-label={t("创建备份实例", "Create backup instance")} aria-busy={pending} onSubmit={event => void submit(event)}>
      {failure && <ErrorState requestId={failure.requestId}>{t("实例未能创建，请检查名称后重试。", "The instance could not be created. Check its name, then retry.")}</ErrorState>}
      <FormField label={t("实例名称", "Instance name")}><InstanceNameField name="name" required readOnly={pending} data-sarmg-initial-focus /></FormField>
      <div className="sarmg-actions"><Button disabled={pending} onClick={close}>{t("取消", "Cancel")}</Button><Button type="submit" disabled={pending}>{pending ? t("正在创建…", "Creating…") : t("创建实例", "Create instance")}</Button></div>
    </form>
  </Dialog>;
}

function InstanceManager({ user, reload }: { user: BackupUser; reload(): void }) {
  const { notify } = useAdminApplication();
  const [pending, setPending] = useState(false);
  const [remove, setRemove] = useState<BackupInstance | null>(null);
  const [failure, setFailure] = useState<{ action: "rotate" | "remove"; requestId?: string } | null>(null);
  async function rotate(instance: BackupInstance) {
    setPending(true); setFailure(null);
    try {
      await request(`/api/v2/admin/instances/${instance.id}/authorization`, isBackupInstance, { method: "PUT" });
      notify(t("授权码已更换，客户端必须重新配对", "Authorization code changed; the client must pair again")); reload();
    } catch (error) {
      setFailure({ action: "rotate", requestId: errorRequestId(error) });
    } finally { setPending(false); }
  }
  async function removeInstance(instance: BackupInstance) {
    setPending(true); setFailure(null);
    try {
      await request(`/api/v2/admin/instances/${instance.id}`, isUndefined, { method: "DELETE" });
      notify(instance.status === "cancelled" || instance.status === "revoked" ? t("实例信息已删除", "Instance entry deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry")); setRemove(null); reload();
    } catch (error) {
      setFailure({ action: "remove", requestId: errorRequestId(error) });
    } finally { setPending(false); }
  }
  return <section className="sarmg-content-panel sarmg-content-stack" aria-label={t("客户端实例", "Client instances")}>
    <h2>{t("客户端配对", "Client pairing")}</h2>
    <p>{t("实例拥有一个长期客户端授权码。服务端加密保存并可查看；更换后旧客户端立即失效并需要重新配对。", "The instance owns one long-lived client authorization code. The server stores it encrypted and keeps it viewable; changing it invalidates the old client and requires pairing again.")}</p>
    {failure?.action === "rotate" && <ErrorState requestId={failure.requestId}>{t("未能更换授权码，请重试。", "Unable to change the authorization code. Please retry.")}</ErrorState>}
    {user.instances.length === 0 ? <EmptyState>{t("这是旧版未完成的记录，没有客户端授权码；请新建备份实例。", "This legacy incomplete entry has no client authorization code. Create a new backup instance.")}</EmptyState> : <Table aria-label={t("客户端配对信息", "Client pairing information")}><thead><tr><th>{t("名称", "Name")}</th><th>{t("状态", "Status")}</th><th>{t("授权码", "Authorization code")}</th><th>{t("平台", "Platform")}</th><th>{t("操作", "Actions")}</th></tr></thead><tbody>{user.instances.map(instance => { const terminal = instance.status === "cancelled" || instance.status === "revoked"; return <tr key={instance.id}><td>{instance.name}</td><td><StatusBadge status={instance.status} /></td><td><code>{instance.authorization_code}</code></td><td>{instance.platform}</td><td>{!terminal && <Button disabled={pending} onClick={() => void rotate(instance)}>{t("更换授权码", "Change code")}</Button>}<Button disabled={pending} onClick={() => { setFailure(null); setRemove(instance); }}>{instance.status === "pending" ? t("取消配对", "Cancel pairing") : terminal ? t("删除实例", "Delete instance") : t("撤销实例", "Revoke instance")}</Button></td></tr>; })}</tbody></Table>}
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
  navigation: [], routes: <Application /> });
const root = document.getElementById("root");
if (root === null) throw new Error("缺少 React 根节点");
createRoot(root).render(<StrictMode><Root /></StrictMode>);
