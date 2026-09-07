/** Shared by React and native Web consumers; no product operations or authentication. */
export type WorkspaceConfig = Readonly<{
    appearance: string;
    layout: "instances" | "custom";
    selection: "underline" | "custom";
    fontFamily: string;
    instanceNameMaxCharacters: number;
    headerControls: "icons" | "text";
    diagnostics: false;
    showVersion: false;
    navigationPlacement: "header";
    headerIconSize: "1em";
    emptyInstanceSidebar: "collapse";
}>;
export declare const DEFAULT_WORKSPACE_CONFIG: WorkspaceConfig;
export declare function resolveWorkspaceConfig(input?: Partial<WorkspaceConfig>): WorkspaceConfig;
export declare function validInstanceName(value: string, maximum?: number): boolean;
export declare const WORKSPACE_ICON_PATHS: Readonly<{
    create: "M12 5v14M5 12h14";
    refresh: "M20 4v6h-6M4 20v-6h6M5.1 9a7 7 0 0 1 11.5-3L20 10M4 14l3.4 4A7 7 0 0 0 18.9 15";
    logout: "M9 4H4v16h5m5-12 4 4-4 4M8 12h12";
    moon: "M20.8 13A9 9 0 0 1 11 3.2 9 9 0 1 0 20.8 13Z";
    sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M5 19l1.4-1.4M17.6 6.4 19 5";
}>;
