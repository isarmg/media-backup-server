import { getLocale, t } from "@xcss/web/admin-ui/i18n";

const pairing: Record<string, readonly [string, string]> = {
  pending: ["待配对", "Awaiting pairing"], paired: ["已配对", "Paired"],
  cancelled: ["已取消", "Cancelled"], revoked: ["已撤销", "Revoked"],
};
const actions: Record<string, readonly [string, string]> = {
  "device.instance.create": ["创建实例", "Create instance"],
  "device.instance.delete": ["删除实例", "Delete instance"],
  "device.instance.revoke": ["撤销实例", "Revoke instance"],
  "device.authorization.rotate": ["更换密码", "Change password"],
  "device.pairing.cancel": ["取消配对", "Cancel pairing"],
  "device.bootstrap": ["配对实例", "Pair instance"],
  "upload.commit": ["完成上传", "Complete upload"],
  "upload.superseded": ["上传已被替代", "Upload superseded"],
  "asset.update": ["更新媒体", "Update media"], "asset.trash": ["移入回收站", "Move to trash"],
  "asset.restore": ["恢复媒体", "Restore media"], "asset.delete": ["删除媒体", "Delete media"],
  "album.sync": ["同步相册", "Sync album"], "tag.create": ["创建标签", "Create tag"],
  "tag.assets.set": ["设置媒体标签", "Set media tags"], "tag.asset.add": ["添加媒体标签", "Add media tag"],
  "tag.asset.remove": ["移除媒体标签", "Remove media tag"],
  "resource.deduplicate": ["复用已有文件", "Reuse existing file"],
  "api_key.create": ["创建 API 密钥", "Create API key"], "api_key.revoke": ["撤销 API 密钥", "Revoke API key"],
};
export function pairingLabel(value: string): string {
  const label = Object.hasOwn(pairing, value) ? pairing[value] : undefined;
  return label ? t(...label) : t("未知", "Unknown");
}
export function actionLabel(value: string): string {
  const label = Object.hasOwn(actions, value) ? actions[value] : undefined;
  return label ? t(...label) : t("未识别的操作", "Unrecognized action");
}
/** SQLite datetime('now') values are UTC even though they omit a zone suffix. */
export function lastSeen(value: string | null | undefined): string {
  if (!value) return t("尚未配对", "Not paired yet");
  const instant = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value) ? value.replace(" ", "T") + "Z" : value;
  const date = new Date(instant);
  return Number.isFinite(date.getTime()) ? date.toLocaleString(getLocale()) : t("未知", "Unknown");
}
