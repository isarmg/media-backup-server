export const DEFAULT_WORKSPACE_CONFIG = Object.freeze({
    appearance: "content-blocks",
    layout: "instances",
    selection: "underline",
    fontFamily: '"Sarmg Maple",ui-monospace,monospace',
    instanceNameMaxCharacters: 32,
    headerControls: "icons",
    diagnostics: false,
    showVersion: false,
    navigationPlacement: "header",
    headerIconSize: "1em",
    emptyInstanceSidebar: "collapse",
});
export function resolveWorkspaceConfig(input = {}) {
    const result = { ...DEFAULT_WORKSPACE_CONFIG, ...input };
    if (!/^[a-z][a-z0-9-]{0,63}$/.test(result.appearance)
        || !["instances", "custom"].includes(result.layout)
        || !["underline", "custom"].includes(result.selection)
        || !["icons", "text"].includes(result.headerControls)
        || result.diagnostics !== false || result.showVersion !== false || result.navigationPlacement !== "header"
        || result.headerIconSize !== "1em"
        || result.emptyInstanceSidebar !== "collapse"
        || !result.fontFamily.trim()
        || !Number.isInteger(result.instanceNameMaxCharacters)
        || result.instanceNameMaxCharacters < 1 || result.instanceNameMaxCharacters > 32) {
        throw new TypeError("Invalid Foundation workspace configuration");
    }
    return Object.freeze(result);
}
export function validInstanceName(value, maximum = DEFAULT_WORKSPACE_CONFIG.instanceNameMaxCharacters) {
    const name = value.trim();
    return maximum >= 1 && maximum <= 32 && name.length > 0 && [...name].length <= maximum && !/[\u0000-\u001f\u007f]/u.test(name);
}
export const WORKSPACE_ICON_PATHS = Object.freeze({
    create: "M12 5v14M5 12h14",
    refresh: "M20 4v6h-6M4 20v-6h6M5.1 9a7 7 0 0 1 11.5-3L20 10M4 14l3.4 4A7 7 0 0 0 18.9 15",
    logout: "M9 4H4v16h5m5-12 4 4-4 4M8 12h12",
    moon: "M20.8 13A9 9 0 0 1 11 3.2 9 9 0 1 0 20.8 13Z",
    sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M5 19l1.4-1.4M17.6 6.4 19 5",
});
