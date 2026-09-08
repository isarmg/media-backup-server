import { type AdministratorApiClient } from "@sarmg/admin-web";
/** Shared self-service account entry for standard shells and custom products. */
export declare function AccountSettings({ client, username, onUpdated }: {
    client: AdministratorApiClient;
    username: string;
    onUpdated?(): void;
}): import("react").JSX.Element;
