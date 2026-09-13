import { t } from "../shell/i18n.js";
import { StrictMode, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { createSarmgAdminApplication, errorRequestId, useAdminApplication, InstancePageNavigation, InstanceHeaderActions, InstanceNameField } from "../shell/index.js";
import { Button, Checkbox, ConfirmDangerDialog, Dialog, EmptyState, ErrorState, FormField, LoadingState, StatusBadge, Table, TextField } from "@sarmg/admin-ui";
import "@sarmg/design-tokens/tokens.css";
import "@sarmg/design-tokens/tokens.dark.css";
import "@sarmg/design-tokens/reset.css";
import "@sarmg/design-tokens/accessibility.css";
import "../fonts/fonts.css";
import "@sarmg/admin-ui/styles.css";
import "./styles.css";
import "../appearance/content-blocks.css";
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
  const [createPending, setCreatePending] = useState(false);
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
    <InstanceHeaderActions create={() => setCreating(true)} createLabel={t("新建备份用户", "Create backup user")} refresh={reload} />
    <InstancePageNavigation page={view} detailsDisabled={!selected} navigate={value => { window.location.hash = value === "details" && selected ? `details/${encodeURIComponent(selected)}` : value; }} />
    <h1 className="sarmg-visually-hidden">{view === "instances" ? t("备份实例列表", "Backup instance list") : view === "details" ? t("备份详细信息", "Backup details") : t("备份日志", "Backup logs")}</h1>
    {failure ? <ErrorState requestId={failure.requestId} onRetry={reload}>{t("备份数据暂不可用，请重试。", "Backup data is temporarily unavailable. Please retry.")}</ErrorState>
      : overview === null ? <LoadingState>{t("正在载入备份数据…", "Loading backup data…")}</LoadingState>
      : view === "instances" ? <OverviewView overview={overview} /> : view === "logs" ? <LogsView /> : <><a href="#instances">{t("返回实例列表", "Back to instance list")}</a>{user ? <UsersView overview={{...overview, users:[user]}} reload={reload} /> : <EmptyState>{t("请选择一个备份实例。", "Select a backup instance.")}</EmptyState>}</>}
    {creating && <Dialog title={t("新建备份用户", "Create backup user")} onClose={() => { if (!createPending) setCreating(false); }}><BackupUserForm pendingChanged={setCreatePending} reload={() => { setCreating(false); reload(); }} /></Dialog>}
  </div>;
}

function OverviewView({ overview }: { overview: Overview }) {
  return <div className="media-sections"><Section title={t("备份统计", "Backup statistics")}><Table aria-label={t("备份统计", "Backup statistics")}>
    <thead><tr><th scope="col">{t("统计项", "Metric")}</th><th scope="col">{t("当前值", "Current value")}</th></tr></thead>
    <tbody>
      <tr><th scope="row">{t("启用 / 全部用户", "Active / total users")}</th><td>{overview.active_users} / {overview.total_users}</td></tr>
      <tr><th scope="row">{t("媒体已用", "Media storage used")}</th><td>{bytes(overview.used_bytes)}</td></tr>
      <tr><th scope="row">{t("上传预留空间", "Reserved upload space")}</th><td>{bytes(overview.pending_bytes)}</td></tr>
      <tr><th scope="row">{t("已分配配额", "Allocated quota")}</th><td>{bytes(overview.quota_bytes)}{overview.unlimited_users > 0 ? t(" + 不限", " + Unlimited") : ""}</td></tr>
    </tbody>
  </Table></Section><Section title={t("用户概览", "User overview")}>
    {overview.users.length === 0 ? <EmptyState>{t("暂无备份用户", "No backup users yet")}</EmptyState> : <div className="media-overview-table"><Table aria-label={t("用户概览", "User overview")}>
      <thead><tr>{[t("用户", "User"), t("账号", "Account"), t("状态", "Status"), t("设备", "Devices"), t("资源", "Resources"), t("已用容量 / 配额", "Used / quota"), t("上传预留", "Upload reservation"), t("存储路径", "Storage path")].map(label => <th scope="col" key={label}>{label}</th>)}</tr></thead>
      <tbody>{overview.users.map(user => <tr key={user.id}>
        <th scope="row"><a href={"#details/" + encodeURIComponent(user.id)}>{user.display_name}</a></th>
        <td>{user.username}</td><td><StatusBadge status={user.enabled ? t("已启用", "Enabled") : t("已停用", "Disabled")} /></td>
        <td>{user.device_count}</td><td>{user.resource_count}</td>
        <td>{bytes(user.used_bytes)} / {user.quota_bytes === 0 ? t("不限", "Unlimited") : bytes(user.quota_bytes)}<progress max={1} value={user.quota_bytes > 0 ? Math.min(1, user.used_bytes / user.quota_bytes) : 0} aria-label={user.username + t(" 存储配额占用比例", " Storage quota usage")} /></td>
        <td>{bytes(user.pending_bytes)}</td><td>{user.storage_path}</td>
      </tr>)}</tbody>
    </Table></div>}
  </Section><Section title={t("实例列表", "Instance list")}>
    {overview.users.every(user => user.instances.length === 0) ? <EmptyState>{t("暂无客户端实例", "No client instances")}</EmptyState> : <Table aria-label={t("实例列表", "Instance list")}>
      <thead><tr><th>{t("实例", "Instance")}</th><th>{t("所属用户", "Owner")}</th><th>{t("状态", "Status")}</th><th>{t("平台", "Platform")}</th><th>{t("最后在线", "Last seen")}</th></tr></thead>
      <tbody>{overview.users.flatMap(user => user.instances.map(instance => <tr key={instance.id}>
        <th scope="row"><a href={"#details/" + encodeURIComponent(user.id)}>{instance.name}</a></th><td>{user.display_name}</td><td><StatusBadge status={instance.status} /></td><td>{instance.platform}</td><td>{instance.last_seen_at || t("尚未配对", "Not paired yet")}</td>
      </tr>))}</tbody>
    </Table>}
  </Section></div>;
}

function UsersView({ overview, reload }: { overview: Overview; reload(): void }) {
  return <div className="media-sections"><p>{t("备份用户用于设备上传。配额为 0 表示不限。", "Backup users upload from devices. A quota of 0 means unlimited.")}</p>
    <Section title={t("管理备份用户", "Manage backup users")}><div className="media-grid">
      {overview.users.length === 0 ? <EmptyState>{t("暂无备份用户", "No backup users yet")}</EmptyState> : overview.users.map(user => <div key={user.id} className="sarmg-content-stack"><BackupUserForm user={user} reload={reload} /><InstanceManager user={user} reload={reload} /></div>)}
    </div></Section></div>;
}

function BackupUserForm({ user, reload, pendingChanged }: { user?: BackupUser; reload(): void; pendingChanged?(value: boolean): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [disableInput, setDisableInput] = useState<Record<string, unknown> | null>(null);
  async function save(input: Record<string, unknown>, form?: HTMLFormElement) {
    if (busy.current) return;
    busy.current = true; setPending(true); pendingChanged?.(true); setFailure(null);
    try {
      await request(user ? "/api/v2/admin/users/" + user.id : "/api/v2/admin/users", isBackupUser,
        { method: user ? "PUT" : "POST", body: JSON.stringify(input) });
      form?.reset(); setDisableInput(null); notify(user ? t("备份用户已保存", "Backup user saved") : t("备份用户已创建", "Backup user created")); reload();
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
    finally { busy.current = false; setPending(false); pendingChanged?.(false); }
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (busy.current) return;
    const form = event.currentTarget, data = new FormData(form); setFailure(null);
    try {
      const input = { username: String(data.get("username") ?? ""), display_name: String(data.get("display_name") ?? ""),
        storage_path: String(data.get("storage_path") ?? ""), quota_bytes: quotaBytes(data.get("quota_gib")),
        enabled: user ? data.has("enabled") : true };
      if (user?.enabled && !input.enabled) setDisableInput(input); else void save(input, form);
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); }
  }
  const error = failure && <ErrorState requestId={failure.requestId}>{t("未能保存备份用户，请检查账号、路径和配额后重试。", "Unable to save the backup user. Check the account, path and quota, then retry.")}</ErrorState>;
  return <article className="media-card sarmg-content-panel">
    {user && <><h3>{user.display_name}</h3><StatusBadge status={user.enabled ? t("已启用", "Enabled") : t("已停用", "Disabled")} /></>}
    <form aria-label={user ? t("编辑备份用户 ", "Edit backup user ") + user.username : t("创建备份用户", "Create backup user")} aria-busy={pending} onSubmit={submit}>
      {!disableInput && error}
      <FormField label={t("名称", "Name")}><InstanceNameField name="display_name" defaultValue={user?.display_name ?? ""} required readOnly={pending} /></FormField>
      <FormField label={t("账号", "Account")}><TextField name="username" defaultValue={user?.username ?? ""} required minLength={3} maxLength={64} readOnly={pending} autoComplete="off" /></FormField>
      <FormField label={t("存储路径", "Storage path")}><TextField name="storage_path" defaultValue={user?.storage_path ?? ""} placeholder={t("自动分配", "Automatically assigned")} required={!!user} readOnly={pending} /></FormField>
      <FormField label={t("配额（GiB，0 表示不限）", "Quota (GiB; 0 means unlimited)")}><TextField name="quota_gib" type="number" min={0} step="any" defaultValue={user ? user.quota_bytes / GIB : 100} required readOnly={pending} /></FormField>
      {user && <label className="media-check"><Checkbox name="enabled" defaultChecked={user.enabled} disabled={pending} />{t("启用备份用户", "Enable backup user")}</label>}
      <div className="sarmg-actions"><Button type="submit" disabled={pending}>{pending ? t("正在保存…", "Saving…") : user ? t("保存备份用户", "Save backup user") : t("创建备份用户", "Create backup user")}</Button></div>
    </form>
    {user && disableInput && <ConfirmDangerDialog title={t("停用备份用户 ", "Disable backup user ") + user.username + "？"} description={t("该账户将无法继续上传，已有备份数据不会删除。", "This account will no longer be able to upload. Existing backups will not be deleted.")} pending={pending}
      onClose={() => { if (!busy.current) { setDisableInput(null); setFailure(null); } }} onConfirm={() => void save(disableInput)}>{error}</ConfirmDangerDialog>}
  </article>;
}

function InstanceManager({ user, reload }: { user: BackupUser; reload(): void }) {
  const { notify } = useAdminApplication();
  const [name, setName] = useState("");
  const [pending, setPending] = useState(false);
  const [remove, setRemove] = useState<BackupInstance | null>(null);
  async function create() {
    setPending(true);
    try {
      await request(`/api/v2/admin/users/${user.id}/instances`, isBackupInstance, { method: "POST", body: JSON.stringify({ name }) });
      setName(""); notify(t("备份实例已创建", "Backup instance created")); reload();
    } finally { setPending(false); }
  }
  async function rotate(instance: BackupInstance) {
    setPending(true);
    try {
      await request(`/api/v2/admin/instances/${instance.id}/authorization`, isBackupInstance, { method: "PUT" });
      notify(t("授权码已更换，客户端必须重新配对", "Authorization code changed; the client must pair again")); reload();
    } finally { setPending(false); }
  }
  async function removeInstance(instance: BackupInstance) {
    setPending(true);
    try {
      await request(`/api/v2/admin/instances/${instance.id}`, isUndefined, { method: "DELETE" });
      notify(instance.status === "cancelled" || instance.status === "revoked" ? t("实例信息已删除", "Instance entry deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry")); reload();
    } finally { setPending(false); setRemove(null); }
  }
  return <section className="sarmg-content-panel sarmg-content-stack" aria-label={t("客户端实例", "Client instances")}>
    <h2>{t("客户端实例", "Client instances")}</h2>
    <p>{t("每个实例只有一个长期授权码。服务端加密保存并可查看；更换后旧客户端立即失效并需要重新配对。", "Each instance has one long-lived authorization code. It is encrypted and viewable on the server; changing it invalidates the old client and requires pairing again.")}</p>
    <FormField label={t("实例名称", "Instance name")}><InstanceNameField value={name} onChange={event => setName(event.currentTarget.value)} /></FormField>
    <div className="sarmg-actions"><Button disabled={pending || !name.trim()} onClick={() => void create()}>{t("创建实例", "Create instance")}</Button></div>
    {user.instances.length === 0 ? <EmptyState>{t("暂无客户端实例", "No client instances")}</EmptyState> : <Table aria-label={t("客户端实例列表", "Client instance list")}><thead><tr><th>{t("名称", "Name")}</th><th>{t("状态", "Status")}</th><th>{t("授权码", "Authorization code")}</th><th>{t("平台", "Platform")}</th><th>{t("操作", "Actions")}</th></tr></thead><tbody>{user.instances.map(instance => { const terminal = instance.status === "cancelled" || instance.status === "revoked"; return <tr key={instance.id}><td>{instance.name}</td><td><StatusBadge status={instance.status} /></td><td><code>{instance.authorization_code}</code></td><td>{instance.platform}</td><td>{!terminal && <Button disabled={pending} onClick={() => void rotate(instance)}>{t("更换授权码", "Change code")}</Button>}<Button disabled={pending} onClick={() => setRemove(instance)}>{instance.status === "pending" ? t("取消配对", "Cancel pairing") : terminal ? t("删除实例", "Delete instance") : t("撤销实例", "Revoke instance")}</Button></td></tr>; })}</tbody></Table>}
    {remove && (() => { const terminal = remove.status === "cancelled" || remove.status === "revoked"; return <ConfirmDangerDialog title={terminal ? t("删除实例", "Delete instance") : t("取消或撤销实例", "Cancel or revoke instance")} description={terminal ? t("永久删除这条终态且没有备份数据的实例信息。", "Permanently delete this terminal instance entry when it owns no backup data.") : t("授权码和访问令牌将立即失效。终态实例之后可从列表永久删除。", "The authorization code and access token are invalidated immediately. The terminal instance can then be permanently deleted from the list.")} pending={pending} onClose={() => { if (!pending) setRemove(null); }} onConfirm={() => void removeInstance(remove)} />; })()}
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
