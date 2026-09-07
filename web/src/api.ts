import { createAdministratorApiClient, type JsonGuard } from "@sarmg/admin-web";

export const administratorApi = createAdministratorApiClient();

export type BackupUser = {
  id: string;
  username: string;
  display_name: string;
  storage_path: string;
  quota_bytes: number;
  used_bytes: number;
  pending_bytes: number;
  device_count: number;
  resource_count: number;
  enabled: boolean;
  created_at: string;
  last_seen_at: string;
};

export type Overview = {
  users: BackupUser[];
  total_users: number;
  active_users: number;
  unlimited_users: number;
  used_bytes: number;
  pending_bytes: number;
  quota_bytes: number;
};

export const isUndefined = (value: unknown): value is undefined => value === undefined;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const isString = (value: unknown): value is string => typeof value === "string";
const isNumber = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value);
const isBoolean = (value: unknown): value is boolean => typeof value === "boolean";

export const isBackupUser: JsonGuard<BackupUser> = (
  value,
): value is BackupUser =>
  isRecord(value) &&
  ["id", "username", "display_name", "storage_path", "created_at", "last_seen_at"].every(
    (key) => isString(value[key]),
  ) &&
  ["quota_bytes", "used_bytes", "pending_bytes", "device_count", "resource_count"].every(
    (key) => isNumber(value[key]),
  ) &&
  isBoolean(value.enabled);

export const isOverview: JsonGuard<Overview> = (value): value is Overview =>
  isRecord(value) &&
  Array.isArray(value.users) &&
  value.users.every(isBackupUser) &&
  [
    "total_users",
    "active_users",
    "unlimited_users",
    "used_bytes",
    "pending_bytes",
    "quota_bytes",
  ].every((key) => isNumber(value[key]));

export function request<T>(
  path: string,
  guard: JsonGuard<T>,
  init?: RequestInit,
): Promise<T> {
  if (!path.startsWith("/api/v2/admin/")) {
    throw new TypeError("Media Backup 管理 API 必须位于 /api/v2/admin/");
  }
  return administratorApi.request(path, guard, init);
}
