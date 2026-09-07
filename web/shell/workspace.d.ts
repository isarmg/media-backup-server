import { type ReactNode, type InputHTMLAttributes } from "react";
import { WORKSPACE_ICON_PATHS } from "./workspace-config.js";
export declare const WorkspaceContext: import("react").Context<Readonly<{
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
}>>;
export declare const HeaderActionsContext: import("react").Context<HTMLElement | null>;
export declare const HeaderNavigationContext: import("react").Context<HTMLElement | null>;
export declare function HeaderNavigation({ children, label }: {
    children: ReactNode;
    label?: string;
}): import("react").ReactPortal | null;
export declare function HeaderActions({ children }: {
    children: ReactNode;
}): import("react").ReactPortal | null;
export declare function WorkspaceIcon({ name }: {
    name: keyof typeof WORKSPACE_ICON_PATHS;
}): import("react").JSX.Element;
export declare function InstanceHeaderActions({ create, refresh, refreshing, createLabel, refreshLabel }: {
    create?: () => void;
    refresh?: () => void;
    refreshing?: boolean;
    createLabel?: string;
    refreshLabel?: string;
}): import("react").JSX.Element;
export declare function InstanceWorkspace({ instances, selected, select, label, showSidebar, children }: {
    instances: readonly {
        id: string;
        name: string;
    }[];
    selected?: string | null;
    select(id: string): void;
    label?: string;
    showSidebar?: boolean;
    children: ReactNode;
}): import("react").JSX.Element;
/** Count Unicode scalar values, matching Rust chars(); do not use UTF-16 maxLength. */
export declare function InstanceNameField({ onChange, onInput, ...props }: Omit<InputHTMLAttributes<HTMLInputElement>, "maxLength">): import("react").JSX.Element;
