import { Button } from "@xcss/admin-ui";
import { t } from "@xcss/admin-ui/i18n";
import type { PageCursors } from "./api";

export function PageNavigation({ page, first, move, label, canRestart = false, loading = false, firstLabel = t("第一页", "First page") }: {
  page: PageCursors | null;
  first(): void;
  move(cursor: string): void;
  label: string;
  canRestart?: boolean;
  loading?: boolean;
  firstLabel?: string;
}) {
  return <nav className="xcss-actions" aria-label={label}>
    <Button disabled={loading || !page || (!canRestart && !page.previous_cursor)} onClick={first}>{firstLabel}</Button>
    <Button disabled={loading || !page?.previous_cursor} onClick={() => { if (page?.previous_cursor) move(page.previous_cursor); }}>{t("上一页", "Previous page")}</Button>
    <Button disabled={loading || !page?.next_cursor} onClick={() => { if (page?.next_cursor) move(page.next_cursor); }}>{t("下一页", "Next page")}</Button>
  </nav>;
}
