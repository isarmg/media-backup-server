import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { t } from "./i18n.js";
import { languageLabel, switchLanguage, validationMessage } from "./i18n.js";
import { Component, createContext, useCallback, useContext, useEffect, useId, useRef, useState, } from "react";
import { Button, ErrorState, FormField, IconButton, LoadingState, PageHeader, TextField, Toast, } from "@sarmg/admin-ui";
import { createAdministratorApiClient, } from "@sarmg/admin-web";
import { useAdministratorSession } from "@sarmg/admin-web/react";
import { WorkspaceContext, HeaderActionsContext, HeaderNavigationContext, WorkspaceIcon } from "./workspace.js";
import { resolveWorkspaceConfig } from "./workspace-config.js";
export { HeaderActions, HeaderNavigation, InstanceHeaderActions, InstanceWorkspace, InstanceNameField, WorkspaceIcon } from "./workspace.js";
export { DEFAULT_WORKSPACE_CONFIG, resolveWorkspaceConfig, validInstanceName } from "./workspace-config.js";
export { AdministratorsPanel } from "./administrators.js";
const Context = createContext(null);
export function useAdminApplication() {
    const context = useContext(Context);
    if (!context)
        throw new Error("Product routes must be inside the administrator application");
    return context;
}
/** Only an opaque validated identifier is rendered; never error.message or stack. */
export function errorRequestId(error) {
    try {
        if (error && typeof error === "object" && "requestId" in error
            && typeof error.requestId === "string" && /^[A-Za-z0-9._:-]{1,128}$/.test(error.requestId))
            return error.requestId;
    }
    catch { /* hostile/unexpected error objects are not public diagnostics */ }
    return undefined;
}
export class ApplicationErrorBoundary extends Component {
    state = { failed: false };
    static getDerivedStateFromError(error) {
        return { failed: true, requestId: errorRequestId(error) };
    }
    componentDidUpdate(previous) {
        if (this.state.failed && previous.resetKey !== this.props.resetKey)
            this.setState({ failed: false, requestId: undefined });
    }
    render() {
        return this.state.failed
            ? _jsx(ErrorState, { requestId: this.state.requestId, onRetry: () => this.setState({ failed: false, requestId: undefined }), children: t("无法显示此页面。", "This page could not be displayed.") })
            : this.props.children;
    }
}
export function createSarmgAdminApplication(options) {
    if (!options.product.name.trim())
        throw new TypeError("Product identity is required");
    const seen = new Set();
    for (const item of options.navigation) {
        if (!item.label.trim() || !/^(?:\/(?!\/)|#)/.test(item.href) || /[\\\u0000-\u0020\u007f]/.test(item.href) || seen.has(item.href)) {
            throw new TypeError("Navigation requires unique local links and labels");
        }
        seen.add(item.href);
    }
    options = { ...options, workspace: resolveWorkspaceConfig(options.workspace) };
    const client = options.client ?? createAdministratorApiClient();
    return function SarmgAdminApplication() {
        return _jsx(ApplicationErrorBoundary, { children: _jsx(AdminShell, { options: options, client: client }) });
    };
}
function AdminShell({ options, client }) {
    const session = useAdministratorSession(client);
    const [logoutPending, setLogoutPending] = useState(false);
    const [logoutError, setLogoutError] = useState(null);
    const [toasts, setToasts] = useState([]);
    const sequence = useRef(0);
    const [headerActions, setHeaderActions] = useState(null);
    const [headerNavigation, setHeaderNavigation] = useState(null);
    const workspace = resolveWorkspaceConfig(options.workspace);
    useEffect(() => {
        const root = document.documentElement;
        const previous = { appearance: root.dataset.sarmgAppearance, selection: root.dataset.sarmgSelection, font: root.style.getPropertyValue("--sarmg-font-ui") };
        root.dataset.sarmgAppearance = workspace.appearance;
        root.dataset.sarmgSelection = workspace.selection;
        root.style.setProperty("--sarmg-font-ui", workspace.fontFamily);
        return () => {
            if (previous.appearance === undefined)
                delete root.dataset.sarmgAppearance;
            else
                root.dataset.sarmgAppearance = previous.appearance;
            if (previous.selection === undefined)
                delete root.dataset.sarmgSelection;
            else
                root.dataset.sarmgSelection = previous.selection;
            if (previous.font)
                root.style.setProperty("--sarmg-font-ui", previous.font);
            else
                root.style.removeProperty("--sarmg-font-ui");
        };
    }, [workspace.appearance, workspace.selection, workspace.fontFamily]);
    const notify = useCallback((message) => {
        const id = ++sequence.current;
        setToasts(current => [...current.slice(-4), { id, message: message.slice(0, 512) }]);
    }, []);
    const [location, setLocation] = useState(() => typeof window === "undefined" ? "" : window.location.pathname + window.location.hash);
    useEffect(() => {
        const changed = () => setLocation(window.location.pathname + window.location.hash);
        window.addEventListener("popstate", changed);
        window.addEventListener("hashchange", changed);
        return () => { window.removeEventListener("popstate", changed); window.removeEventListener("hashchange", changed); };
    }, []);
    useEffect(() => {
        if (session.phase !== "authenticated") {
            setToasts([]);
            setLogoutError(null);
        }
    }, [session.phase]);
    const identity = _jsx("div", { className: "sarmg-product-identity", children: _jsx("strong", { children: options.product.name }) });
    if (session.phase !== "authenticated") {
        return _jsxs("div", { className: "sarmg-auth-shell", children: [_jsx("div", { className: "sarmg-auth-language", style: { position: "absolute", insetBlockStart: "1rem", insetInlineEnd: "1rem" }, children: _jsx(LanguageToggle, {}) }), _jsxs("div", { className: "sarmg-auth-card", children: [identity, session.phase === "loading" ? _jsx(LoadingState, { children: t("正在恢复管理员会话…", "Restoring administrator session…") })
                            : session.phase === "error" ? _jsx(ErrorState, { requestId: errorRequestId(session.error), onRetry: () => void session.restore(), children: t("无法恢复管理员会话。", "Unable to restore administrator session.") })
                                : _jsx(LoginPage, { login: session.login })] })] });
    }
    async function logout() {
        setLogoutPending(true);
        setLogoutError(null);
        try {
            await session.logout();
        }
        catch (error) {
            setLogoutError(error);
        }
        finally {
            setLogoutPending(false);
        }
    }
    return _jsx(Context.Provider, { value: { client, session: session.session, notify }, children: _jsxs("div", { className: "sarmg-admin-shell", style: { "--sarmg-header-icon-size": workspace.headerIconSize }, children: [_jsx("a", { className: "sarmg-skip-link", href: "#sarmg-main-content", onClick: event => {
                        event.preventDefault();
                        document.getElementById("sarmg-main-content")?.focus();
                    }, children: t("跳至正文", "Skip to content") }), _jsxs(PageHeader, { children: [_jsx("div", { className: "sarmg-header-navigation-slot", children: _jsxs("div", { className: "sarmg-header-brand-navigation", children: [identity, _jsx("div", { ref: setHeaderNavigation, style: { display: "contents" }, children: options.navigation.length > 0 && _jsx("nav", { className: "sarmg-header-navigation", "aria-label": t("产品导航", "Product navigation"), children: options.navigation.map(item => _jsx("a", { href: item.href, "aria-current": (item.href.startsWith("#") ? location.endsWith(item.href) : location === item.href) ? "page" : undefined, children: item.label }, item.href)) }) })] }) }), _jsxs("div", { className: "sarmg-header-actions", role: "group", "aria-label": t("全局操作", "Global actions"), children: [_jsx("div", { ref: setHeaderActions, style: { display: "contents" } }), _jsx(LanguageToggle, {}), _jsx(ThemeToggle, {}), _jsx(IconButton, { disabled: logoutPending, "aria-label": logoutPending ? t("正在退出…", "Signing out…") : t("退出", "Sign out"), title: logoutPending ? t("正在退出…", "Signing out…") : t("退出", "Sign out"), onClick: () => void logout(), children: workspace.headerControls === "icons" ? _jsx(WorkspaceIcon, { name: "logout" }) : t("退出", "Sign out") })] })] }), toasts.length > 0 && _jsx("div", { className: "sarmg-toast-stack", role: "region", "aria-label": t("通知", "Notifications"), children: toasts.map(toast => _jsxs(Toast, { children: [_jsx("span", { children: toast.message }), _jsx(IconButton, { "aria-label": t("关闭通知", "Dismiss notification"), onClick: () => setToasts(current => current.filter(item => item.id !== toast.id)), children: "\u00D7" })] }, toast.id)) }), _jsx("div", { className: "sarmg-shell-layout sarmg-shell-layout--full", children: _jsxs("main", { id: "sarmg-main-content", className: "sarmg-shell-main", tabIndex: -1, children: [logoutError !== null && _jsx(ErrorState, { requestId: errorRequestId(logoutError), children: t("无法确认退出结果，请重试。", "Sign out could not be confirmed. Try again.") }), _jsx(ApplicationErrorBoundary, { resetKey: location, children: _jsx(WorkspaceContext.Provider, { value: workspace, children: _jsx(HeaderNavigationContext.Provider, { value: headerNavigation, children: _jsx(HeaderActionsContext.Provider, { value: headerActions, children: options.routes }) }) }) })] }) })] }) });
}
export function LoginPage({ login }) {
    const [failure, setFailure] = useState(null);
    const errorId = useId();
    const [pending, setPending] = useState(false);
    const submitting = useRef(false);
    async function submit(event) {
        event.preventDefault();
        if (submitting.current)
            return;
        const form = event.currentTarget;
        for (const name of ["username", "password"]) {
            const input = form.elements.namedItem(name);
            if (input instanceof HTMLInputElement && !input.validity.valid) {
                const message = input.validity.valueMissing
                    ? name === "username" ? t("请输入用户名。", "Enter your username.") : t("请输入密码。", "Enter your password.")
                    : validationMessage(input);
                setFailure({ message, field: name });
                input.focus();
                return;
            }
        }
        const data = new FormData(form);
        submitting.current = true;
        setPending(true);
        setFailure(null);
        try {
            await login(String(data.get("username") ?? ""), String(data.get("password") ?? ""));
        }
        catch (error) {
            setFailure({ message: t("登录失败，请检查用户名和密码后重试。", "Sign in failed. Check your credentials and try again."), requestId: errorRequestId(error) });
            const password = form.elements.namedItem("password");
            if (password instanceof HTMLInputElement) {
                password.value = "";
                password.focus();
            }
        }
        finally {
            submitting.current = false;
            setPending(false);
        }
    }
    return _jsxs("form", { noValidate: true, onInvalid: event => event.preventDefault(), onInput: () => setFailure(null), onSubmit: event => void submit(event), "aria-busy": pending, children: [_jsx("h1", { children: t("管理员登录", "Administrator sign in") }), _jsx(FormField, { label: t("用户名", "Username"), children: _jsx(TextField, { name: "username", autoComplete: "username", required: true, maxLength: 64, readOnly: pending, "aria-invalid": failure?.field === "username" || undefined, "aria-describedby": failure ? errorId : undefined }) }), _jsx(FormField, { label: t("密码", "Password"), children: _jsx(TextField, { name: "password", type: "password", autoComplete: "current-password", required: true, maxLength: 1024, readOnly: pending, "aria-invalid": failure?.field === "password" || undefined, "aria-describedby": failure ? errorId : undefined }) }), failure && _jsx(ErrorState, { requestId: failure.requestId, children: _jsx("span", { id: errorId, children: failure.message }) }), _jsx(Button, { type: "submit", disabled: pending, children: pending ? t("正在登录…", "Signing in…") : t("登录", "Sign in") })] });
}
function LanguageToggle() {
    return _jsx(IconButton, { "aria-label": languageLabel(), title: languageLabel(), onClick: switchLanguage, children: _jsx("svg", { "aria-hidden": "true", width: "1em", height: "1em", viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "1.7", strokeLinecap: "round", strokeLinejoin: "round", children: _jsx("path", { d: "M3 5h12M9 3v2M6 5c0 5 4 9 8 11M13 5c0 5-4 9-9 12m10 4 4-10 4 10m-6.5-4h5" }) }) });
}
function ThemeToggle() {
    const [theme, setTheme] = useState(() => typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    useEffect(() => {
        const root = document.documentElement;
        const previous = root.dataset.theme;
        root.dataset.theme = theme;
        return () => { if (previous === undefined)
            delete root.dataset.theme;
        else
            root.dataset.theme = previous; };
    }, [theme]);
    const label = theme === "light" ? t("切换到深色模式", "Switch to dark mode") : t("切换到浅色模式", "Switch to light mode");
    return _jsx(IconButton, { "aria-label": label, title: label, onClick: () => setTheme(current => current === "light" ? "dark" : "light"), children: _jsx("svg", { "aria-hidden": "true", width: "22", height: "22", viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "1.7", strokeLinecap: "round", strokeLinejoin: "round", children: theme === "light" ? _jsx("path", { d: "M20.8 13A9 9 0 0 1 11 3.2 9 9 0 1 0 20.8 13Z" }) : _jsxs(_Fragment, { children: [_jsx("circle", { cx: "12", cy: "12", r: "4" }), _jsx("path", { d: "M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M5 19l1.4-1.4M17.6 6.4 19 5" })] }) }) });
}
