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
  created_at: string;
  last_seen_at: string;
  instances: BackupInstance[];
};

export type BackupInstance = {
  id: string;
  name: string;
  platform: string;
  status: string;
  online: boolean;
  authorization_code: string;
  created_at: string;
  last_seen_at: string;
};

export type AdminLog = { sequence: number; action: string; entity_id: string; occurred_at: string };
export type AdminLogs = { date: string; logs: AdminLog[] };

export type Overview = {
  users: BackupUser[];
  total_users: number;
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

export const isAdminLogs: JsonGuard<AdminLogs> = (value): value is AdminLogs =>
  isRecord(value) && isString(value.date) && /^\d{4}-\d{2}-\d{2}$/.test(value.date) &&
  Array.isArray(value.logs) && value.logs.every((item: unknown) =>
    isRecord(item) && isNumber(item.sequence) && isString(item.action) &&
    isString(item.entity_id) && isString(item.occurred_at));

export const isBackupInstance: JsonGuard<BackupInstance> = (value): value is BackupInstance =>
  isRecord(value) && ["id", "name", "platform", "status", "authorization_code", "created_at", "last_seen_at"].every(key => isString(value[key])) && /^[a-z0-9]{36}$/.test(value.authorization_code as string) && isBoolean(value.online);

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
  Array.isArray(value.instances) && value.instances.every(isBackupInstance);


export const isOverview: JsonGuard<Overview> = (value): value is Overview =>
  isRecord(value) &&
  Array.isArray(value.users) &&
  value.users.every(isBackupUser) &&
  [
    "total_users",
    "unlimited_users",
    "used_bytes",
    "pending_bytes",
    "quota_bytes",
  ].every((key) => isNumber(value[key]));

export function request<T>(
  path: string,
  guard: JsonGuard<T>,
  init?: RequestInit & { maxResponseBytes?: number; timeoutMs?: number },
): Promise<T> {
  if (!path.startsWith("/api/v2/admin/")) {
    throw new TypeError("Media Backup 管理 API 必须位于 /api/v2/admin/");
  }
  return administratorApi.request(path, guard, init);
}
