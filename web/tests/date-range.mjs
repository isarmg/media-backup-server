import assert from "node:assert/strict";
import { expect } from "@playwright/test";
export function rangeEditor(page, id) {
  const root = page.locator(`#${id}`);
  const fields = ["year", "month", "day"];
  return { root,
    async draft(start, end = start) {
      for (const [endpoint, date] of [["start", start], ["end", end]]) {
        const parts = date.split("-");
        for (const [index, field] of fields.entries()) await root.locator(`#${id}-${endpoint}-${field}`).fill(index === 0 ? parts[index] ?? "" : parts[index] ? String(Number(parts[index])) : "");
      }
    },
    async apply() { await root.locator("input").last().press("Enter"); },
    async fill(start, end = start) { await this.draft(start, end); await this.apply(); },
    async expectValue(start, end = start) {
      for (const [endpoint, date] of [["start", start], ["end", end]]) {
        for (const [index, field] of fields.entries()) await expect(root.locator(`#${id}-${endpoint}-${field}`)).toHaveValue(index === 0 ? date.split("-")[index] : String(Number(date.split("-")[index])));
      }
    },
  };
}
export async function checkDateRangeValidation(editor, requests, start, end) {
  const before = requests();
  await editor.draft("2022-02-30", "2023-02-02");
  await expect(editor.root.locator('input[aria-invalid="true"]')).toHaveCount(1);
  await expect(editor.root.locator('input[aria-invalid="true"]')).toHaveAttribute("id", /start-day$/);
  await expect(editor.root.getByRole("alert")).toContainText("有效的年月日");
  const colors = await editor.root.evaluate(root => ({ bad: getComputedStyle(root.querySelector('[aria-invalid="true"]')).color, separators: [...root.querySelectorAll('.xcss-date-range-separator')].map(node => getComputedStyle(node).color) }));
  assert.ok(colors.separators.every(color => color !== colors.bad), "slashes and range separator keep their normal color");
  await editor.apply();
  await editor.draft("2024-13-01", "2025-01-01");
  await expect(editor.root.locator('input[aria-invalid="true"]')).toHaveCount(1);
  await expect(editor.root.locator('input[aria-invalid="true"]')).toHaveAttribute("id", /start-month$/);
  await editor.apply();
  await editor.draft("0000-01-01", "2025-01-01");
  await expect(editor.root.locator('input[aria-invalid="true"]')).toHaveCount(1);
  await editor.apply();
  await editor.draft("2023-02-02", "2022-02-01");
  await expect(editor.root.locator('input[aria-invalid="true"]')).toHaveCount(6);
  await expect(editor.root.getByRole("alert")).toContainText("结束日期不能早于");
  await editor.apply();
  assert.equal(requests(), before, "invalid dates never request logs");
  await editor.draft(start, end);
  await expect(editor.root.getByRole("alert")).toHaveCount(0);
  assert.equal(requests(), before, "editing a valid range waits for Enter");
  await editor.apply();
  await expect.poll(requests).toBeGreaterThan(before);
}
