import { t } from "../shell/i18n.js";
import { StrictMode, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { AdministratorsPanel, createSarmgAdminApplication, errorRequestId, useAdminApplication, HeaderNavigation, InstanceHeaderActions, InstanceWorkspace, InstanceNameField } from "../shell/index.js";
import { Button, Checkbox, ConfirmDangerDialog, Dialog, EmptyState, ErrorState, FormField, LoadingState, StatusBadge, Table, TextField } from "@sarmg/admin-ui";
import "@sarmg/design-tokens/tokens.css";
import "@sarmg/design-tokens/tokens.dark.css";
import "@sarmg/design-tokens/reset.css";
import "@sarmg/design-tokens/accessibility.css";
import "../fonts/fonts.css";
import "@sarmg/admin-ui/styles.css";
import "./styles.css";
import "../appearance/content-blocks.css";
import { administratorApi, isBackupUser, isOverview, isUndefined, request, type BackupUser, type Overview } from "./api";

type Failure = { requestId?: string };
type View = "overview" | "users" | "administrators";
const GIB = 1_073_741_824;
function currentView(): View {
  const hash = window.location.hash.slice(1);
  return hash === "users" || hash === "administrators" ? hash : "overview";
}
function quotaBytes(value: FormDataEntryValue | null): number {
  const text = String(value ?? "").trim(), result = Number(text) * GIB;
  if (text === "" || !Number.isSafeInteger(result) || result < 0) throw new Error("Invalid quota");
  return result;
}
function clearPassword(form: HTMLFormElement) {
  const field = form.elements.namedItem("password");
  if (field instanceof HTMLInputElement) { field.value = ""; field.focus(); }
}

function Application() {
  const [view, setView] = useState<View>(currentView);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [generation, setGeneration] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [createPending, setCreatePending] = useState(false);
  const reload = () => setGeneration(value => value + 1);
  useEffect(() => {
    const changed = () => setView(currentView()); window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  useEffect(() => {
    const controller = new AbortController(); setOverview(null); setFailure(null);
    void request("/api/v2/admin/overview", isOverview, { signal: controller.signal })
      .then(value => { if (!controller.signal.aborted) setOverview(value); })
      .catch(error => { if (!controller.signal.aborted) setFailure({ requestId: errorRequestId(error) }); });
    return () => controller.abort();
  }, [generation, view]);
  const user = overview?.users.find(item => item.id === selected) ?? overview?.users[0];
  return <div className="media-business">
    <InstanceHeaderActions create={() => setCreating(true)} createLabel={t("新建备份用户", "Create backup user")} refresh={reload} />
    <InstanceWorkspace instances={overview?.users.map(item => ({ id: item.id, name: item.display_name })) ?? []} selected={user?.id} select={setSelected} label={t("备份用户实例", "Backup user instances")} showSidebar={view === "users"}>
    <HeaderNavigation label={t("备份管理功能", "Backup management navigation")}>{[["overview",t("总览", "Overview")],["users",t("备份用户", "Backup users")],["administrators",t("平台管理员", "Platform administrators")]].map(([id,name]) => <Button key={id} aria-pressed={view === id} onClick={() => { window.location.hash = id!; }}>{name}</Button>)}</HeaderNavigation>
    <h1 className="sarmg-visually-hidden">{view === "overview" ? t("备份总览", "Backup overview") : view === "users" ? t("备份用户", "Backup users") : t("平台管理员", "Platform administrators")}</h1>
    {view === "administrators" ? <AdministratorsPanel />
      : failure ? <ErrorState requestId={failure.requestId} onRetry={reload}>{t("备份数据暂不可用，请重试。", "Backup data is temporarily unavailable. Please retry.")}</ErrorState>
      : overview === null ? <LoadingState>{t("正在载入备份数据…", "Loading backup data…")}</LoadingState>
      : view === "overview" ? <OverviewView overview={overview} /> : <UsersView overview={{...overview, users:user ? [user] : []}} reload={reload} />}
    </InstanceWorkspace>
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
  </Table></Section><Section title={t("用户概览", "User overview")}><div className="media-grid">
    {overview.users.length === 0 ? <EmptyState>{t("暂无备份用户", "No backup users yet")}</EmptyState> : overview.users.map(user => <article className="media-card sarmg-content-panel" key={user.id}>
      <h3>{user.display_name}</h3><StatusBadge status={user.enabled ? t("已启用", "Enabled") : t("已停用", "Disabled")} />
      <dl><dt>{t("账号", "Account")}</dt><dd>{user.username}</dd><dt>{t("设备", "Devices")}</dt><dd>{user.device_count}</dd>
        <dt>{t("资源", "Resources")}</dt><dd>{user.resource_count}</dd><dt>{t("容量", "Storage")}</dt><dd>{bytes(user.used_bytes)} / {user.quota_bytes === 0 ? t("不限", "Unlimited") : bytes(user.quota_bytes)}</dd>
        <dt>{t("上传预留", "Upload reservation")}</dt><dd>{bytes(user.pending_bytes)}</dd><dt>{t("存储路径", "Storage path")}</dt><dd>{user.storage_path}</dd></dl>
      <progress max={1} value={user.quota_bytes > 0 ? Math.min(1, user.used_bytes / user.quota_bytes) : 0} aria-label={user.username + t(" 存储配额占用比例", " Storage quota usage")} />
    </article>)}
  </div></Section></div>;
}

function UsersView({ overview, reload }: { overview: Overview; reload(): void }) {
  return <div className="media-sections"><p>{t("备份用户用于设备上传，与平台管理员账户相互独立。配额为 0 表示不限。", "Backup users upload from devices and are separate from platform administrators. A quota of 0 means unlimited.")}</p>
    <Section title={t("管理备份用户", "Manage backup users")}><div className="media-grid">
      {overview.users.length === 0 ? <EmptyState>{t("暂无备份用户", "No backup users yet")}</EmptyState> : overview.users.map(user => <BackupUserForm key={user.id} user={user} reload={reload} />)}
    </div></Section></div>;
}

function BackupUserForm({ user, reload, pendingChanged }: { user?: BackupUser; reload(): void; pendingChanged?(value: boolean): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [password, setPassword] = useState(false);
  const [disableInput, setDisableInput] = useState<Record<string, unknown> | null>(null);
  async function save(input: Record<string, unknown>, form?: HTMLFormElement) {
    if (busy.current) return;
    busy.current = true; setPending(true); pendingChanged?.(true); setFailure(null);
    try {
      await request(user ? "/api/v2/admin/users/" + user.id : "/api/v2/admin/users", isBackupUser,
        { method: user ? "PUT" : "POST", body: JSON.stringify(input) });
      form?.reset(); setDisableInput(null); notify(user ? t("备份用户已保存", "Backup user saved") : t("备份用户已创建", "Backup user created")); reload();
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); if (form) clearPassword(form); }
    finally { busy.current = false; setPending(false); pendingChanged?.(false); }
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (busy.current) return;
    const form = event.currentTarget, data = new FormData(form); setFailure(null);
    try {
      const input = { username: String(data.get("username") ?? ""), display_name: String(data.get("display_name") ?? ""),
        storage_path: String(data.get("storage_path") ?? ""), quota_bytes: quotaBytes(data.get("quota_gib")),
        enabled: user ? data.has("enabled") : true, ...(user ? {} : { password: String(data.get("password") ?? "") }) };
      if (user?.enabled && !input.enabled) setDisableInput(input); else void save(input, form);
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); clearPassword(form); }
  }
  const error = failure && <ErrorState requestId={failure.requestId}>{t("未能保存备份用户，请检查账号、路径和配额后重试。", "Unable to save the backup user. Check the account, path and quota, then retry.")}</ErrorState>;
  return <article className="media-card sarmg-content-panel">
    {user && <><h3>{user.display_name}</h3><StatusBadge status={user.enabled ? t("已启用", "Enabled") : t("已停用", "Disabled")} /></>}
    <form aria-label={user ? t("编辑备份用户 ", "Edit backup user ") + user.username : t("创建备份用户", "Create backup user")} aria-busy={pending} onSubmit={submit}>
      {!disableInput && error}
      <FormField label={t("名称", "Name")}><InstanceNameField name="display_name" defaultValue={user?.display_name ?? ""} required readOnly={pending} /></FormField>
      <FormField label={t("账号", "Account")}><TextField name="username" defaultValue={user?.username ?? ""} required minLength={3} maxLength={64} readOnly={pending} autoComplete="off" /></FormField>
      {!user && <FormField label={t("密码", "Password")}><TextField name="password" type="password" required minLength={12} maxLength={128} readOnly={pending} autoComplete="new-password" /></FormField>}
      <FormField label={t("存储路径", "Storage path")}><TextField name="storage_path" defaultValue={user?.storage_path ?? ""} placeholder={t("自动分配", "Automatically assigned")} required={!!user} readOnly={pending} /></FormField>
      <FormField label={t("配额（GiB，0 表示不限）", "Quota (GiB; 0 means unlimited)")}><TextField name="quota_gib" type="number" min={0} step="any" defaultValue={user ? user.quota_bytes / GIB : 100} required readOnly={pending} /></FormField>
      {user && <label className="media-check"><Checkbox name="enabled" defaultChecked={user.enabled} disabled={pending} />{t("启用备份用户", "Enable backup user")}</label>}
      <div className="sarmg-actions"><Button type="submit" disabled={pending}>{pending ? t("正在保存…", "Saving…") : user ? t("保存备份用户", "Save backup user") : t("创建备份用户", "Create backup user")}</Button>
        {user && <Button disabled={pending} onClick={() => setPassword(true)}>{t("重设备份密码", "Reset backup password")}</Button>}</div>
    </form>
    {user && disableInput && <ConfirmDangerDialog title={t("停用备份用户 ", "Disable backup user ") + user.username + "？"} description={t("该账户将无法继续上传，已有备份数据不会删除。", "This account will no longer be able to upload. Existing backups will not be deleted.")} pending={pending}
      onClose={() => { if (!busy.current) { setDisableInput(null); setFailure(null); } }} onConfirm={() => void save(disableInput)}>{error}</ConfirmDangerDialog>}
    {user && password && <BackupPasswordDialog user={user} close={() => setPassword(false)} />}
  </article>;
}

function BackupPasswordDialog({ user, close }: { user: BackupUser; close(): void }) {
  const { notify } = useAdminApplication();
  const busy = useRef(false);
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<Failure | null>(null);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); if (busy.current) return;
    const form = event.currentTarget, data = new FormData(form);
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request("/api/v2/admin/users/" + user.id + "/reset-password", isUndefined,
        { method: "POST", body: JSON.stringify({ password: String(data.get("password") ?? "") }) });
      form.reset(); notify(t("备份用户密码已重设", "Backup user password reset")); close();
    } catch (error) { setFailure({ requestId: errorRequestId(error) }); clearPassword(form); }
    finally { busy.current = false; setPending(false); }
  }
  return <Dialog title={t("重设备份密码 · ", "Reset backup password · ") + user.username} onClose={() => { if (!busy.current) close(); }}>
    <form aria-busy={pending} onSubmit={event => void submit(event)}>
      {failure && <ErrorState requestId={failure.requestId}>{t("未能重设密码，请重试。", "Unable to reset the password. Please retry.")}</ErrorState>}
      <FormField label={t("新备份密码", "New backup password")}><TextField name="password" type="password" minLength={12} maxLength={128} required autoComplete="new-password" readOnly={pending} /></FormField>
      <div className="sarmg-actions"><Button disabled={pending} onClick={close}>{t("取消", "Cancel")}</Button><Button type="submit" disabled={pending}>{t("确认重设密码", "Confirm password reset")}</Button></div>
    </form>
  </Dialog>;
}

function Section({ title, children }: { title: string; children: ReactNode }) { return <section><h2>{title}</h2>{children}</section>; }
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
