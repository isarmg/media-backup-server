import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useRef, useState } from "react";
import { Button, Dialog, ErrorState, FormField, IconButton, TextField } from "@sarmg/admin-ui";
import { t } from "./i18n.js";
import {} from "@sarmg/admin-web";
/** Shared self-service account entry for standard shells and custom products. */
export function AccountSettings({ client, username, onUpdated }) {
    const [open, setOpen] = useState(false);
    const [pending, setPending] = useState(false);
    const [failure, setFailure] = useState(null);
    const busy = useRef(false);
    async function submit(event) {
        event.preventDefault();
        if (busy.current)
            return;
        const form = event.currentTarget;
        const data = new FormData(form);
        const password = String(data.get("new_password") ?? "");
        if (password !== String(data.get("confirm_password") ?? "")) {
            setFailure(t("两次输入的新密码不一致。", "The new passwords do not match."));
            return;
        }
        busy.current = true;
        setPending(true);
        setFailure(null);
        try {
            await client.updateAccount({ username: String(data.get("username") ?? ""), current_password: String(data.get("current_password") ?? ""), ...(password ? { new_password: password } : {}) });
            form.reset();
            setOpen(false);
            onUpdated?.();
        }
        catch (error) {
            const code = error && typeof error === "object" && "code" in error ? error.code : null;
            setFailure(code === "admin.current_password_invalid"
                ? t("当前密码不正确。", "The current password is incorrect.")
                : code === "admin.conflict" ? t("此账号名称已被使用。", "This account name is already in use.")
                    : t("账号未能更新，请检查输入并重试。", "The account could not be updated. Check the input and retry."));
            for (const name of ["current_password", "new_password", "confirm_password"]) {
                const input = form.elements.namedItem(name);
                if (input instanceof HTMLInputElement)
                    input.value = "";
            }
        }
        finally {
            busy.current = false;
            setPending(false);
        }
    }
    return _jsxs(_Fragment, { children: [_jsx(IconButton, { "aria-label": t("账号设置", "Account settings"), title: t("账号设置", "Account settings"), style: { borderRadius: "50%", aspectRatio: "1" }, onClick: () => { setFailure(null); setOpen(true); }, children: _jsxs("svg", { viewBox: "0 0 24 24", width: "22", height: "22", fill: "none", stroke: "currentColor", strokeWidth: "1.7", "aria-hidden": "true", children: [_jsx("circle", { cx: "12", cy: "8", r: "3.5" }), _jsx("path", { d: "M4.5 21v-2a7.5 7.5 0 0 1 15 0v2" })] }) }), open && _jsx(Dialog, { title: t("账号设置", "Account settings"), onClose: () => { if (!busy.current)
                    setOpen(false); }, children: _jsxs("form", { onSubmit: submit, "aria-busy": pending, children: [_jsx("p", { children: t("修改当前登录账号；保存后需重新登录。新密码留空则保留现有密码。", "Update the signed-in account. Sign in again after saving. Leave the new password blank to keep it.") }), _jsx(FormField, { label: t("账号名称", "Account name"), children: _jsx(TextField, { name: "username", autoComplete: "username", defaultValue: username, required: true, minLength: 3, maxLength: 64, disabled: pending }) }), _jsx(FormField, { label: t("当前密码", "Current password"), children: _jsx(TextField, { name: "current_password", type: "password", autoComplete: "current-password", required: true, maxLength: 1024, disabled: pending }) }), _jsx(FormField, { label: t("新密码", "New password"), children: _jsx(TextField, { name: "new_password", type: "password", autoComplete: "new-password", maxLength: 1024, disabled: pending }) }), _jsx(FormField, { label: t("确认新密码", "Confirm new password"), children: _jsx(TextField, { name: "confirm_password", type: "password", autoComplete: "new-password", maxLength: 1024, disabled: pending }) }), failure && _jsx(ErrorState, { children: failure }), _jsxs("div", { className: "sarmg-actions", children: [_jsx(Button, { type: "submit", disabled: pending, children: pending ? t("正在保存…", "Saving…") : t("保存账号", "Save account") }), _jsx(Button, { type: "button", disabled: pending, onClick: () => setOpen(false), children: t("取消", "Cancel") })] })] }) })] });
}
