import { startAfterFonts } from "@xcss/web/web-fonts";
import { t } from "@xcss/web/admin-ui/i18n";
import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { createXcssAdminApplication, errorRequestId, useAdminApplication, InstancePageNavigation, InstanceHeaderActions, AccountPage } from "@xcss/web/admin-shell";
import { EmptyState, ErrorState, LoadingState } from "@xcss/web/admin-ui";
import "@xcss/web/design-tokens/tokens.css";
import "@xcss/web/design-tokens/tokens.dark.css";
import "@xcss/web/design-tokens/reset.css";
import "@xcss/web/design-tokens/accessibility.css";
import "@xcss/web/web-fonts/fonts.css";
import "@xcss/web/admin-ui/styles.css";
import "./styles.css";
import { administratorApi, isBackupInstance, isBackupUser, request, type BackupUser } from "./api";

import { LogsView } from "./LogsView";
import { OverviewView } from "./OverviewView";
import { UserDetailsView } from "./UserDetailsView";

type Failure = { requestId?: string };
type View = "instances" | "details" | "logs" | "account";
function currentView(): View {
  const hash = window.location.hash.slice(1);
  if (hash === "account") return "account";
  return hash.startsWith("details/") ? "details" : hash === "logs" || hash.startsWith("logs/") ? "logs" : "instances";
}
function selectedUserFromLocation(): string | null {
  const match = /^#(?:details|logs)\/(.+)$/.exec(window.location.hash);
  if (!match) return null;
  try { return decodeURIComponent(match[1]); } catch { return null; }
}

function Application() {
  const [view, setView] = useState<View>(currentView);
  const [user, setUser] = useState<BackupUser | null>(null);
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
      await request("/api/v1/admin/instances", isBackupInstance, { method: "POST", body: JSON.stringify({ name: t("新实例", "New instance") }) });
      reload(); notify(t("备份实例已创建，可在详细信息中查看密码", "Backup instance created. Its password is available in details"));
    } catch (error) { setCreateFailure({ requestId: errorRequestId(error) }); }
    finally { setCreating(false); }
  }
  const { notify } = useAdminApplication();
  useEffect(() => {
    const changed = () => { setView(currentView()); const nextSelected = selectedUserFromLocation(); if (currentView() !== "account") setSelected(nextSelected); }; window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  useEffect(() => {
    if (view === "instances" || view === "account" || selected === null) return;
    const controller = new AbortController(); setUser(null); setFailure(null);
    void request(`/api/v1/admin/users/${encodeURIComponent(selected)}`, (value): value is BackupUser => isBackupUser(value) && value.id === selected,
      {signal:controller.signal, maxResponseBytes:1024*1024, timeoutMs:10_000})
      .then(value => { if (!controller.signal.aborted) setUser(value); })
      .catch(error => { if (!controller.signal.aborted) setFailure({requestId:errorRequestId(error)}); });
    return () => controller.abort();
  }, [generation, view, selected]);
  return <div className="media-business xcss-content-stack">
    <InstanceHeaderActions create={() => void createInstance()} refresh={reload} refreshing={creating} />
    <InstancePageNavigation page={view} detailsDisabled={!selected} navigate={value => { window.location.hash = value === "details" && selected ? `details/${encodeURIComponent(selected)}` : value === "logs" && selected ? `logs/${encodeURIComponent(selected)}` : value; }} />
    <h1 className="xcss-visually-hidden">{view === "account" ? t("账号设置", "Account settings") : view === "instances" ? t("备份实例列表", "Backup instance list") : view === "details" ? t("备份详细信息", "Backup details") : t("备份日志", "Backup logs")}</h1>
    {view !== "account" && createFailure && <ErrorState requestId={createFailure.requestId}>{t("实例未能创建，请刷新列表核对后重试。", "The instance could not be created. Refresh the list before retrying.")}</ErrorState>}
    {view === "account" ? <AccountPage key={generation} /> : view === "instances" ? <OverviewView refreshGeneration={generation} /> : view === "logs" && selected === null ? <LogsView refreshGeneration={generation} instance={null} /> : failure ? <ErrorState requestId={failure.requestId} onRetry={reload}>{t("备份数据暂不可用，请重试。", "Backup data is temporarily unavailable. Please retry.")}</ErrorState>
      : user === null ? <LoadingState>{t("正在载入备份数据…", "Loading backup data…")}</LoadingState>
      : view === "logs" ? user?.instances[0] ? <LogsView key={user.instances[0].id} refreshGeneration={generation} instance={user.instances[0]} /> : <EmptyState>{t("所选实例不存在。", "The selected instance does not exist.")}</EmptyState> : user ? <UserDetailsView key={user.id} user={user} reload={reload} /> : <EmptyState>{t("请选择一个备份实例。", "Select a backup instance.")}</EmptyState>}
  </div>;
}


const Root = createXcssAdminApplication({ product: { name: "xszs" }, client: administratorApi,
  navigation: [], loginLandingHref: "#instances", routes: <Application /> });
const root = document.getElementById("root");
if (root === null) throw new Error("The React root element is missing");
void startAfterFonts(() => {
  createRoot(root).render(<StrictMode><Root /></StrictMode>);
});
