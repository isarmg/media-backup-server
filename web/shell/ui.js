import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { t } from "./i18n.js";
import { validationMessage } from "./i18n.js";
import { useEffect, useId, useRef, } from "react";
function classes(base, extra) { return extra ? `${base} ${extra}` : base; }
export function Button({ type = "button", className, ...props }) {
    return _jsx("button", { ...props, type: type, className: classes("sarmg-button", className) });
}
export function IconButton({ "aria-label": label, ...props }) {
    if (!label?.trim())
        throw new TypeError("IconButton requires aria-label");
    return _jsx(Button, { ...props, "aria-label": label });
}
export function TextField({ className, onInvalid, onInput, ...props }) {
    return _jsx("input", { ...props, className: classes("sarmg-input", className), onInvalid: event => {
            if (!event.currentTarget.validity.customError)
                event.currentTarget.setCustomValidity(validationMessage(event.currentTarget));
            onInvalid?.(event);
        }, onInput: event => { event.currentTarget.setCustomValidity(""); onInput?.(event); } });
}
export function Select({ className, onInvalid, onChange, ...props }) {
    return _jsx("select", { ...props, className: classes("sarmg-input", className), onInvalid: event => {
            if (!event.currentTarget.validity.customError)
                event.currentTarget.setCustomValidity(validationMessage(event.currentTarget));
            onInvalid?.(event);
        }, onChange: event => { event.currentTarget.setCustomValidity(""); onChange?.(event); } });
}
export function Checkbox(props) {
    return _jsx("input", { ...props, type: "checkbox" });
}
/** Native modal semantics supply focus containment, background inertness and Escape. */
export function Dialog({ title, description, children, onClose }) {
    const reference = useRef(null);
    const titleId = useId();
    const descriptionId = useId();
    useEffect(() => {
        const dialog = reference.current;
        const previous = document.activeElement;
        dialog.showModal();
        // React's autoFocus runs before showModal and is not a native autofocus
        // attribute. Apply the safe initial target after the dialog is opened.
        dialog.querySelector("[data-sarmg-initial-focus]")?.focus();
        return () => {
            dialog.close();
            if (previous instanceof HTMLElement && previous.isConnected)
                previous.focus();
        };
    }, []);
    return _jsxs("dialog", { ref: reference, className: "sarmg-dialog", "aria-labelledby": titleId, tabIndex: -1, "aria-describedby": description ? descriptionId : undefined, onKeyDown: event => {
            if (event.key !== "Tab")
                return;
            const dialog = reference.current;
            const focusable = Array.from(dialog.querySelectorAll('button,input,select,textarea,a[href],[tabindex]')).filter(element => element.tabIndex >= 0 && !element.matches(':disabled,[hidden]') && element.getClientRects().length > 0);
            const first = focusable[0];
            const last = focusable.at(-1);
            if (!first) {
                event.preventDefault();
                dialog.focus();
            }
            else if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog)) {
                event.preventDefault();
                last.focus();
            }
            else if (!event.shiftKey && document.activeElement === last) {
                event.preventDefault();
                first.focus();
            }
        }, onCancel: (event) => { event.preventDefault(); onClose(); }, children: [_jsxs("div", { className: "sarmg-dialog-heading", children: [_jsx("h2", { id: titleId, children: title }), _jsx(IconButton, { "aria-label": t("关闭对话框", "Close dialog"), onClick: onClose, children: "\u00D7" })] }), description && _jsx("p", { id: descriptionId, children: description }), children] });
}
export function ConfirmDangerDialog({ title, description, onConfirm, onClose, pending = false, children }) {
    return _jsxs(Dialog, { title: title, description: description, onClose: () => { if (!pending)
            onClose(); }, children: [children, _jsxs("div", { className: "sarmg-actions", children: [_jsx(Button, { "data-sarmg-initial-focus": true, disabled: pending, onClick: onClose, children: t("取消", "Cancel") }), _jsx(Button, { className: "sarmg-danger", disabled: pending, onClick: onConfirm, children: pending ? t("正在处理…", "Working…") : t("确认", "Confirm") })] })] });
}
export function Toast({ className, ...props }) {
    return _jsx("div", { ...props, role: "status", "aria-live": "polite", "aria-atomic": "true", className: classes("sarmg-toast", className) });
}
export function StatusBadge({ status }) {
    return _jsx("span", { className: "sarmg-status", children: status });
}
export function Table({ className, ...props }) {
    return _jsx("div", { className: "sarmg-table-scroll", tabIndex: 0, role: "region", "aria-label": props["aria-label"] ?? t("数据表格", "Data table"), children: _jsx("table", { ...props, className: classes("sarmg-table", className) }) });
}
export function FormField({ label, children }) {
    return _jsxs("label", { className: "sarmg-form-field", children: [_jsx("span", { children: label }), children] });
}
export function EmptyState({ children }) {
    return _jsx("div", { className: "sarmg-empty", children: children });
}
export function RequestId({ value }) {
    return value && /^[A-Za-z0-9._:-]{1,128}$/.test(value)
        ? _jsxs("p", { className: "sarmg-request-id", children: [t("请求标识：", "Request ID:"), _jsx("code", { children: value })] }) : null;
}
export function ErrorState({ children, requestId, onRetry }) {
    return _jsxs("div", { className: "sarmg-error", role: "alert", children: [_jsx("div", { children: children }), _jsx(RequestId, { value: requestId }), onRetry && _jsx(Button, { onClick: onRetry, children: t("重试", "Try again") })] });
}
export function LoadingState({ children = t("正在加载…", "Loading…") }) {
    return _jsx("div", { className: "sarmg-loading", role: "status", "aria-live": "polite", children: children });
}
export function PageHeader({ children }) {
    return _jsx("header", { className: "sarmg-page-header", children: children });
}
