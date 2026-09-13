import { t } from "./i18n.js";
import { createLanguageControl } from "./i18n.js";
import { resolveWorkspaceConfig, WORKSPACE_ICON_PATHS, validInstanceName } from "./workspace-config.js";
/** Native Web adapter: retains existing product listeners, including logout/CSRF. */
export function configureNativeWorkspace({ header, content, actions, create, logout, refresh, instanceName, instanceHref, labels = {}, config: input }) {
    const config = resolveWorkspaceConfig(input);
    header.style.setProperty("--sarmg-header-icon-size", config.headerIconSize);
    if (!validInstanceName(instanceName, config.instanceNameMaxCharacters))
        throw new TypeError("Invalid instance name");
    if (!/^\/(?!\/)/.test(instanceHref) || /[\\\u0000-\u0020\u007f]/.test(instanceHref))
        throw new TypeError("Instance target must be local");
    document.documentElement.dataset.sarmgAppearance = config.appearance;
    document.documentElement.dataset.sarmgSelection = config.selection;
    document.documentElement.style.setProperty("--sarmg-font-ui", config.fontFamily);
    actions.classList.add("sarmg-header-actions");
    actions.setAttribute("role", "group");
    actions.setAttribute("aria-label", labels.actions ?? t("全局操作", "Global actions"));
    const icon = (button, name, label) => {
        button.classList.add("sarmg-button");
        button.setAttribute("aria-label", label);
        button.title = label;
        button.querySelector("svg")?.remove();
        button.querySelector("[data-workspace-label]")?.remove();
        if (config.headerControls === "text" && name !== "moon" && name !== "sun") {
            const text = document.createElement("span");
            text.dataset.workspaceLabel = "";
            text.textContent = label;
            button.prepend(text);
            return;
        }
        const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        for (const [key, value] of Object.entries({ width: "22", height: "22", viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.7", "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true" }))
            svg.setAttribute(key, value);
        svg.style.fill = "none";
        const path = document.createElementNS(svg.namespaceURI, "path");
        path.setAttribute("d", WORKSPACE_ICON_PATHS[name]);
        svg.append(path);
        button.prepend(svg);
    };
    if (create) {
        icon(create, "create", create.getAttribute("aria-label") ?? t("新建实例", "Create instance"));
        actions.append(create);
    }
    const reload = document.createElement("button");
    reload.type = "button";
    icon(reload, "refresh", labels.refresh ?? t("刷新", "Refresh"));
    reload.addEventListener("click", refresh);
    actions.append(reload);
    const theme = document.createElement("button");
    theme.type = "button";
    actions.append(createLanguageControl());
    let dark = matchMedia("(prefers-color-scheme: dark)").matches;
    const update = () => { document.documentElement.dataset.theme = dark ? "dark" : "light"; icon(theme, dark ? "sun" : "moon", dark ? (labels.light ?? t("切换到浅色模式", "Switch to light mode")) : (labels.dark ?? t("切换到深色模式", "Switch to dark mode"))); };
    update();
    theme.addEventListener("click", () => { dark = !dark; update(); });
    actions.append(theme);
    icon(logout, "logout", labels.logout ?? logout.getAttribute("aria-label") ?? t("退出", "Sign out"));
    logout.querySelectorAll("span:not([data-workspace-label])").forEach(node => node.hidden = true);
    actions.append(logout);
    if (config.layout === "instances") {
        const workspace = document.createElement("div");
        workspace.className = "sarmg-instance-workspace sarmg-native-workspace";
        const sidebar = document.createElement("aside");
        sidebar.className = "sarmg-instance-sidebar";
        sidebar.setAttribute("aria-label", labels.instances ?? t("共享根实例", "Shared root instance"));
        const list = document.createElement("div");
        list.className = "sarmg-instance-list";
        const link = document.createElement("a");
        link.href = instanceHref;
        link.className = "sarmg-button";
        link.textContent = instanceName;
        link.title = instanceName;
        link.setAttribute("aria-current", "page");
        list.append(link);
        sidebar.append(list);
        content.before(workspace);
        workspace.append(sidebar, content);
        content.classList.add("sarmg-instance-detail");
    }
    header.append(actions);
}
