import { t } from "@xcss/web/admin-ui/i18n";
import { useRef, useState, type FormEvent } from "react";
import { errorRequestId, useAdminApplication, InstanceNameField } from "@xcss/web/admin-shell";
import { Button, ConfirmDangerDialog, ErrorState, FormField, StatusBadge, TextField } from "@xcss/web/admin-ui";
import { isBackupInstance, isBackupUser, isUndefined, request, type BackupInstance, type BackupUser } from "./api";
import { lastSeen, pairingLabel } from "./display-labels";

type Failure = { requestId?: string };
const GIB = 1_073_741_824;
function quotaBytes(value: FormDataEntryValue | null): number {
  const text = String(value ?? "").trim();
  const gib = Number(text);
  const bytes = Math.round(gib * GIB);
  if (text === "" || !Number.isFinite(gib) || gib < 0 || (gib > 0 && bytes === 0) || !Number.isSafeInteger(bytes)) {
    throw new Error("Invalid quota");
  }
  return bytes;
}
export function UserDetailsView({ user, reload }: { user: BackupUser; reload(): void }) {
  return <div className="xcss-content-stack"><PairingAccount user={user} /><BackupStatus user={user} /><BackupUserForm user={user} reload={reload} /><InstanceManager user={user} reload={reload} /></div>;
}

function PairingAccount({ user }: { user: BackupUser }) {
  const instance = user.instances[0];
  if (!instance) return null;
  return <section className="xcss-content-panel" aria-label={t("配对账户信息", "Pairing account information")}><h2>{user.display_name}</h2>
    <dl className="media-detail-list">
      <dt>{t("实例名称", "Instance name")}</dt><dd>{user.display_name}</dd>
      <dt>{t("实例 ID", "Instance ID")}</dt><dd><code>{instance.id}</code></dd>
      <dt>{t("密码", "Password")}</dt><dd><code>{instance.authorization_code}</code></dd>
      <dt>{t("配对状态", "Pairing status")}</dt><dd><StatusBadge status={pairingLabel(instance.status)} /></dd>
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
      await request("/api/v1/admin/users/" + user.id, isBackupUser,
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
  return <section className="media-card xcss-content-panel" aria-label={t("实例设置", "Instance settings")}>
    <h2>{t("实例设置", "Instance settings")}</h2>
    <form aria-label={t("编辑备份实例 ", "Edit backup instance ") + user.display_name} aria-busy={pending} onSubmit={submit}>
      {error}
      <FormField label={t("实例名称", "Instance name")}><InstanceNameField name="display_name" defaultValue={user.display_name} required readOnly={pending} /></FormField>
      <input type="hidden" name="username" value={user.username} />
      <FormField label={t("存储路径", "Storage path")}><TextField name="storage_path" defaultValue={user.storage_path} required readOnly={pending} /></FormField>
      <FormField label={t("配额（GiB，0 表示不限）", "Quota (GiB; 0 means unlimited)")}><TextField name="quota_gib" type="number" min={0} step="any" defaultValue={user.quota_bytes / GIB} required readOnly={pending} /></FormField>
      <div className="xcss-actions"><Button type="submit" disabled={pending}>{pending ? t("正在保存…", "Saving…") : t("保存设置", "Save settings")}</Button></div>
    </form>
  </section>;
}

function BackupStatus({ user }: { user: BackupUser }) {
  return <section className="xcss-content-panel xcss-content-stack" aria-label={t("备份状态", "Backup status")}>
    <h2>{t("备份状态", "Backup status")}</h2>
    {user.instances.map(instance => <div key={instance.id}><dl className="media-detail-list"><dt>{t("客户端", "Client")}</dt><dd>{instance.name}</dd><dt>{t("平台", "Platform")}</dt><dd>{instance.platform}</dd><dt>{t("在线状态", "Online status")}</dt><dd>{instance.online ? t("在线", "Online") : t("离线", "Offline")}</dd><dt>{t("最后在线", "Last seen")}</dt><dd>{lastSeen(instance.last_seen_at)}</dd></dl></div>)}
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
      await request(`/api/v1/admin/instances/${instance.id}/authorization`, isBackupInstance, { method: "PUT" });
      setRotating(null);
      notify(t("密码已更换，客户端必须重新配对", "Password changed; the client must pair again")); reload();
    } catch (error) {
      setFailure({ action: "rotate", requestId: errorRequestId(error) });
    } finally { busy.current = false; setPending(false); }
  }
  async function removeInstance(instance: BackupInstance) {
    if (busy.current) return;
    busy.current = true; setPending(true); setFailure(null);
    try {
      await request(`/api/v1/admin/instances/${instance.id}`, isUndefined, { method: "DELETE" });
      notify(instance.status === "cancelled" || instance.status === "revoked" ? t("实例信息已删除", "Instance entry deleted") : t("实例已取消或撤销，可再次删除其信息", "Instance cancelled or revoked; delete it again to remove its entry")); setRemove(null); reload();
    } catch (error) {
      setFailure({ action: "remove", requestId: errorRequestId(error) });
    } finally { busy.current = false; setPending(false); }
  }
  return <section className="xcss-content-panel xcss-content-stack" aria-label={t("实例操作", "Instance actions")}>
    <h2>{t("实例操作", "Instance actions")}</h2>
    <p>{t("实例拥有一个长期客户端密码。服务端加密保存并可查看；更换后旧客户端立即失效并需要重新配对。", "The instance owns one long-lived client password. The server stores it encrypted and keeps it viewable; changing it invalidates the old client and requires pairing again.")}</p>
    {user.instances.length > 0 && user.instances.map(instance => { const terminal = instance.status === "cancelled" || instance.status === "revoked"; return <div className="xcss-content-stack" key={instance.id}>{user.instances.length > 1 && <h3>{instance.name}</h3>}<div className="xcss-actions">{!terminal && <Button disabled={pending} onClick={() => { setFailure(null); setRotating(instance); }}>{t("更换密码", "Change password")}</Button>}<Button disabled={pending} onClick={() => { setFailure(null); setRemove(instance); }}>{instance.status === "pending" ? t("取消配对", "Cancel pairing") : terminal ? t("删除实例", "Delete instance") : t("撤销实例", "Revoke instance")}</Button></div></div>; })}
    {rotating && <ConfirmDangerDialog title={t("更换密码", "Change password")} description={t("更换密码会立即撤销当前客户端凭据。客户端必须使用新密码重新配对。", "Changing the password immediately revokes the current client credential. The client must pair again using the new password.")} pending={pending} onClose={() => { if (!busy.current) { setRotating(null); setFailure(null); } }} onConfirm={() => void rotate(rotating)}>{failure?.action === "rotate" && <ErrorState requestId={failure.requestId}>{t("更换结果未能确认，请关闭此窗口并刷新实例信息，核对密码后再操作。", "The change could not be confirmed. Close this dialog and refresh the instance details to check the password before continuing.")}</ErrorState>}</ConfirmDangerDialog>}
    {remove && (() => { const terminal = remove.status === "cancelled" || remove.status === "revoked"; return <ConfirmDangerDialog title={terminal ? t("删除实例", "Delete instance") : t("取消或撤销实例", "Cancel or revoke instance")} description={terminal ? t("永久删除这条终态且没有备份数据的实例信息。", "Permanently delete this terminal instance entry when it owns no backup data.") : t("密码和访问令牌将立即失效。终态实例之后可从列表永久删除。", "The password and access token are invalidated immediately. The terminal instance can then be permanently deleted from the list.")} pending={pending} onClose={() => { if (!pending) { setRemove(null); setFailure(null); } }} onConfirm={() => void removeInstance(remove)}>{failure?.action === "remove" && <ErrorState requestId={failure.requestId}>{terminal ? t("实例仍有备份记录或暂时无法删除，请处理后重试。", "The instance still owns backup records or cannot currently be deleted. Resolve the issue and retry.") : t("未能取消或撤销实例，请重试。", "Unable to cancel or revoke the instance. Please retry.")}</ErrorState>}</ConfirmDangerDialog>; })()}
  </section>;
}
