export const LANGUAGE_STORAGE_KEY = "sarmg.admin.language";
export function resolveLocale() {
    if (typeof window === "undefined" || typeof window.location?.href !== "string")
        return "en";
    const query = new URL(window.location.href).searchParams.get("lang");
    if (query === "zh-CN" || query === "en")
        return query;
    try {
        const saved = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
        if (saved === "zh-CN" || saved === "en")
            return saved;
    }
    catch { /* Storage may be unavailable in private or restricted browsers. */ }
    return window.navigator?.language?.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}
// A document uses one language, even when another tab changes the preference.
const documentLocale = resolveLocale();
export function getLocale() { return documentLocale; }
export function t(zh, en, values = []) {
    const message = getLocale() === "zh-CN" ? zh : en;
    return message.replace(/\{(\d+)\}/g, (token, index) => values[Number(index)] === undefined ? token : String(values[Number(index)]));
}
export function initializeLanguage() {
    if (typeof document === "undefined" || !document.documentElement)
        return;
    document.documentElement.lang = getLocale();
    // Commit the preference only in the new document. A cancelled beforeunload
    // must not change the language of the old document or its future messages.
    if (typeof window !== "undefined" && window.location?.href) {
        const requested = new URL(window.location.href).searchParams.get("lang");
        if (requested === "zh-CN" || requested === "en") {
            try {
                window.localStorage.setItem(LANGUAGE_STORAGE_KEY, getLocale());
            }
            catch { /* URL remains the fallback. */ }
        }
    }
}
/** Reload deliberately: module-level messages and native clients share one locale.
 * Language choice is an immediate navigation, never a second confirmation flow.
 */
export function switchLanguage() {
    const locale = getLocale() === "zh-CN" ? "en" : "zh-CN";
    const url = new URL(window.location.href);
    url.searchParams.set("lang", locale);
    window.location.assign(url.href);
}
export function languageLabel() { return t("切换为英文", "Switch to Chinese"); }
export function validationMessage(input) {
    const validity = input.validity;
    if (validity.valueMissing)
        return t("请填写此项。", "Please fill out this field.");
    if (validity.typeMismatch)
        return t("请输入有效的格式。", "Please enter a valid value.");
    if (validity.tooLong)
        return t("输入内容过长。", "The value is too long.");
    if (validity.tooShort)
        return t("输入内容过短。", "The value is too short.");
    if (validity.rangeOverflow || validity.rangeUnderflow)
        return t("输入值超出允许范围。", "The value is outside the allowed range.");
    if (validity.patternMismatch)
        return t("输入内容不符合要求的格式。", "Please match the required format.");
    if (validity.stepMismatch || validity.badInput)
        return t("请输入允许范围内的有效数字。", "Please enter a valid number within the allowed range.");
    return t("请检查输入内容。", "Please check this value.");
}
export function createLanguageControl() {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "sarmg-button";
    button.setAttribute("aria-label", languageLabel());
    button.title = languageLabel();
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [name, value] of Object.entries({ width: "1em", height: "1em", viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.7", "aria-hidden": "true" }))
        svg.setAttribute(name, value);
    const path = document.createElementNS(svg.namespaceURI, "path");
    path.setAttribute("d", "M3 5h12M9 3v2M6 5c0 5 4 9 8 11M13 5c0 5-4 9-9 12m10 4 4-10 4 10m-6.5-4h5");
    svg.append(path);
    button.append(svg);
    button.addEventListener("click", switchLanguage);
    return button;
}
initializeLanguage();
