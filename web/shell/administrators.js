import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { t } from "./i18n.js";
import { Table } from "@sarmg/admin-ui";
import { useAdminApplication } from "./index.js";
/** Account information for consumers that retain a system settings page. */
export function AdministratorsPanel() {
    const { session } = useAdminApplication();
    return _jsxs("section", { children: [_jsx("h2", { children: t("管理员账号", "Administrator account") }), _jsxs(Table, { "aria-label": t("管理员账号", "Administrator account"), children: [_jsx("thead", { children: _jsxs("tr", { children: [_jsx("th", { scope: "col", children: t("账号名称", "Account name") }), _jsx("th", { scope: "col", children: t("角色", "Role") })] }) }), _jsx("tbody", { children: _jsxs("tr", { children: [_jsx("th", { scope: "row", children: session.username }), _jsx("td", { children: t("管理员", "Administrator") })] }) })] }), _jsx("p", { children: t("本系统仅允许一个管理员。修改账号名称和密码，请使用右上角人物图标。", "This system has one administrator. Use the person icon at the top right to change the account name or password.") })] });
}
