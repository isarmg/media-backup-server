import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { t, getLocale } from "./i18n.js";
import { useEffect, useMemo, useRef, useState } from "react";
import { createAdministratorManagementClient } from "@sarmg/admin-web";
import { Button, ConfirmDangerDialog, Dialog, EmptyState, ErrorState, FormField, LoadingState, PageHeader, StatusBadge, Table, TextField } from "@sarmg/admin-ui";
import { errorRequestId, useAdminApplication } from "./index.js";
const LIMIT = 50;
/** Mount only in a persistent-administrator product's business routes. */
export function AdministratorsPanel() {
    const { client, session, notify } = useAdminApplication();
    const management = useMemo(() => createAdministratorManagementClient(client), [client]);
    const [records, setRecords] = useState(null);
    const [failure, setFailure] = useState(null);
    const [generation, setGeneration] = useState(0);
    const [offset, setOffset] = useState(0);
    const [editor, setEditor] = useState(null);
    const [pending, setPending] = useState(false);
    const submitting = useRef(false);
    const [mutationFailure, setMutationFailure] = useState(null);
    useEffect(() => {
        const controller = new AbortController();
        setRecords(null);
        setFailure(null);
        void management.list(LIMIT, offset, controller.signal)
            .then(value => { if (!controller.signal.aborted)
            setRecords(value); })
            .catch(error => { if (!controller.signal.aborted)
            setFailure({ requestId: errorRequestId(error) }); });
        return () => controller.abort();
    }, [management, offset, generation]);
    const open = (value) => { setMutationFailure(null); setEditor(value); };
    const close = () => { if (!submitting.current) {
        setEditor(null);
        setMutationFailure(null);
    } };
    async function mutate(operation, selfRevocation, form) {
        if (submitting.current)
            return;
        submitting.current = true;
        setPending(true);
        setMutationFailure(null);
        try {
            await operation();
            setEditor(null);
            setGeneration(value => value + 1);
            notify(selfRevocation ? t("管理员已更新，请重新登录。", "Administrator updated. Sign in again.") : t("管理员已更新。", "Administrator updated."));
            if (selfRevocation) {
                await client.restore().catch(() => undefined);
            }
        }
        catch (error) {
            setMutationFailure({ requestId: errorRequestId(error) });
            const password = form?.elements.namedItem("password");
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
    function submit(event) {
        event.preventDefault();
        if (!editor || editor.kind === "disable")
            return;
        const form = event.currentTarget;
        const data = new FormData(form);
        const password = String(data.get("password") ?? "");
        if (editor.kind === "create") {
            void mutate(() => management.create(String(data.get("username") ?? ""), password), false, form);
        }
        else {
            const administrator = editor.administrator;
            void mutate(() => management.setPassword(administrator.administrator_id, password), administrator.administrator_id === session.user_id, form);
        }
    }
    return _jsxs("section", { "aria-label": t("管理员管理", "Administrator management"), children: [_jsxs(PageHeader, { children: [_jsx("h2", { children: t("管理员", "Administrators") }), _jsx(Button, { onClick: () => open({ kind: "create" }), children: t("创建管理员", "Create administrator") })] }), _jsx("p", { children: t("所有账户均具有管理员权限。修改密码或停用账户会撤销该账户的所有会话。不能停用最后一位有效管理员。", "All accounts have administrator access. Password changes and disabling revoke every session for that account. The final active administrator cannot be disabled.") }), failure ? _jsx(ErrorState, { requestId: failure.requestId, onRetry: () => setGeneration(value => value + 1), children: t("无法加载管理员列表。", "Administrators could not be loaded.") })
                : records === null ? _jsx(LoadingState, { children: t("正在加载管理员…", "Loading administrators…") })
                    : records.length === 0 ? _jsx(EmptyState, { children: t("本页暂无管理员。", "No administrators on this page.") })
                        : _jsxs(Table, { "aria-label": t("管理员", "Administrators"), children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { scope: "col", children: t("用户名", "Username") }), _jsx("th", { scope: "col", children: t("状态", "Status") }), _jsx("th", { scope: "col", children: t("最近登录", "Last sign in") }), _jsx("th", { scope: "col", children: t("操作", "Actions") })] }) }), _jsx("tbody", { children: records.map(record => _jsxs("tr", { children: [_jsxs("th", { scope: "row", children: [record.username, record.administrator_id === session.user_id ? t("（本人）", " (you)") : ""] }), _jsx("td", { children: _jsx(StatusBadge, { status: record.active ? t("有效", "Active") : t("已停用", "Disabled") }) }), _jsx("td", { children: record.last_login_at_micros === null ? t("从未登录", "Never") : new Date(record.last_login_at_micros / 1000).toLocaleString(getLocale()) }), _jsx("td", { children: _jsxs("div", { className: "sarmg-actions", children: [_jsx(Button, { disabled: !record.active, "aria-label": t("修改 {0} 的密码", "Change password for {0}", [record.username]), onClick: () => open({ kind: "password", administrator: record }), children: t("修改密码", "Change password") }), _jsx(Button, { disabled: !record.active, "aria-label": t("停用 {0}", "Disable {0}", [record.username]), onClick: () => open({ kind: "disable", administrator: record }), children: t("停用", "Disable") })] }) })] }, record.administrator_id)) })] }), _jsxs("nav", { className: "sarmg-actions", "aria-label": t("管理员分页", "Administrator pages"), children: [_jsx(Button, { disabled: offset === 0 || records === null, onClick: () => setOffset(value => Math.max(0, value - LIMIT)), children: t("上一页管理员", "Previous administrators") }), _jsx("span", { children: t("第 {0} 页", "Page {0}", [offset / LIMIT + 1]) }), _jsx(Button, { disabled: records === null || records.length < LIMIT, onClick: () => setOffset(value => value + LIMIT), children: t("下一页管理员", "Next administrators") })] }), editor?.kind === "disable" ? _jsx(ConfirmDangerDialog, { title: t("停用 {0}？", "Disable {0}?", [editor.administrator.username]), description: t("此操作会撤销该账户的所有会话。停用的账户不能登录。", "This revokes every session for this account. Disabled accounts cannot sign in."), pending: pending, onClose: close, onConfirm: () => { const administrator = editor.administrator; void mutate(() => management.disable(administrator.administrator_id), administrator.administrator_id === session.user_id); }, children: mutationFailure && _jsx(ErrorState, { requestId: mutationFailure.requestId, children: t("无法停用管理员，必须保留至少一位有效管理员。", "Administrator could not be disabled. The final active administrator must remain enabled.") }) }) : editor && _jsx(Dialog, { title: editor.kind === "create" ? t("创建管理员", "Create administrator") : t("修改 {0} 的密码", "Change password for {0}", [editor.administrator.username]), onClose: close, children: _jsxs("form", { onSubmit: submit, "aria-busy": pending, children: [mutationFailure && _jsx(ErrorState, { requestId: mutationFailure.requestId, children: t("无法更新管理员，请检查输入后重试。", "Administrator could not be updated. Check the input and try again.") }), editor.kind === "create" && _jsx(FormField, { label: t("用户名", "Username"), children: _jsx(TextField, { name: "username", required: true, minLength: 3, maxLength: 64, autoComplete: "off", readOnly: pending }) }), _jsx(FormField, { label: t("新密码", "New password"), children: _jsx(TextField, { name: "password", type: "password", required: true, minLength: 12, maxLength: 1024, autoComplete: "new-password", readOnly: pending }) }), _jsxs("div", { className: "sarmg-actions", children: [_jsx(Button, { disabled: pending, onClick: close, children: t("取消", "Cancel") }), _jsx(Button, { type: "submit", disabled: pending, children: pending ? t("正在保存…", "Saving…") : t("保存管理员", "Save administrator") })] })] }) })] });
}
