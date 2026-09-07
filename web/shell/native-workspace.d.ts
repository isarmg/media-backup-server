import { type WorkspaceConfig } from "./workspace-config.js";
/** Native Web adapter: retains existing product listeners, including logout/CSRF. */
export declare function configureNativeWorkspace({ header, content, actions, create, logout, refresh, instanceName, instanceHref, labels, config: input }: {
    header: HTMLElement;
    content: HTMLElement;
    actions: HTMLElement;
    create?: HTMLButtonElement;
    logout: HTMLButtonElement;
    refresh(): void;
    instanceName: string;
    instanceHref: string;
    config?: Partial<WorkspaceConfig>;
    labels?: Partial<{
        actions: string;
        refresh: string;
        light: string;
        dark: string;
        logout: string;
        instances: string;
    }>;
}): void;
