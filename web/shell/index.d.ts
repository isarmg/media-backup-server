import { Component, type ReactNode } from "react";
import { type AdministratorApiClient } from "@sarmg/admin-web";
import { type AdministratorSessionController } from "@sarmg/admin-web/react";
import { type WorkspaceConfig } from "./workspace-config.js";
export { HeaderActions, HeaderNavigation, InstanceHeaderActions, InstanceWorkspace, InstanceNameField, WorkspaceIcon } from "./workspace.js";
export { DEFAULT_WORKSPACE_CONFIG, resolveWorkspaceConfig, validInstanceName } from "./workspace-config.js";
export type { WorkspaceConfig } from "./workspace-config.js";
export { AdministratorsPanel } from "./administrators.js";
export type ProductIdentity = {
    name: string;
};
export type NavigationItem = {
    label: string;
    href: string;
};
export type AdminApplicationOptions = {
    product: ProductIdentity;
    client?: AdministratorApiClient;
    navigation: readonly NavigationItem[];
    routes: ReactNode;
    workspace?: Partial<WorkspaceConfig>;
};
type ApplicationContext = {
    client: AdministratorApiClient;
    session: NonNullable<AdministratorSessionController["session"]>;
    notify(message: string): void;
};
export declare function useAdminApplication(): ApplicationContext;
/** Only an opaque validated identifier is rendered; never error.message or stack. */
export declare function errorRequestId(error: unknown): string | undefined;
export declare class ApplicationErrorBoundary extends Component<{
    children: ReactNode;
    resetKey?: string;
}, {
    failed: boolean;
    requestId?: string;
}> {
    state: {
        failed: boolean;
        requestId?: string;
    };
    static getDerivedStateFromError(error: unknown): {
        failed: boolean;
        requestId: string | undefined;
    };
    componentDidUpdate(previous: {
        resetKey?: string;
    }): void;
    render(): string | number | bigint | boolean | Iterable<ReactNode> | Promise<string | number | bigint | boolean | import("react").ReactPortal | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | null | undefined> | import("react").JSX.Element | null | undefined;
}
export declare function createSarmgAdminApplication(options: AdminApplicationOptions): () => import("react").JSX.Element;
export declare function LoginPage({ login }: {
    login: (username: string, password: string) => Promise<void>;
}): import("react").JSX.Element;
