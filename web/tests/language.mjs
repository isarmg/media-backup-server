import assert from "node:assert/strict";
import { expect } from "@playwright/test";

/** Run against product-owned API fixtures. Exempt only fixture names, not UI text. */
export async function checkWebLanguage(page, { routes, names = [] }) {
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  const chineseUrl = page.url();
  await page.getByRole("button", { name: "切换为英文", exact: true }).click();
  const confirmation = page.getByRole("dialog", { name: "切换语言", exact: true });
  await expect(confirmation.getByRole("button", { name: "取消", exact: true })).toBeFocused();
  await confirmation.getByRole("button", { name: "确认", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.getByRole("button", { name: "Switch to Chinese", exact: true })).toBeVisible();
  for (const [hash, label] of routes) {
    await page.getByRole("banner").getByRole("button", { name: label, exact: true }).click();
    await expect.poll(() => new URL(page.url()).hash).toBe("#" + hash);
    const text = await page.locator("body").innerText();
    const authored = names.reduce((value, name) => value.replaceAll(name, ""), text);
    assert.doesNotMatch(authored, /\p{Script=Han}/u, "English page must not contain untranslated Chinese UI text");
    const attributes = await page.locator("[aria-label], [title], [placeholder]").evaluateAll(nodes => nodes.flatMap(node => ["aria-label", "title", "placeholder"].map(name => node.getAttribute(name) || "")).join("\n"));
    assert.doesNotMatch(names.reduce((value, name) => value.replaceAll(name, ""), attributes), /\p{Script=Han}/u);
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  }
  // Storage preference survives a URL without a language parameter.
  const url = new URL(page.url()); url.searchParams.delete("lang");
  await page.goto(url.href); await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await page.getByRole("button", { name: "Switch to Chinese", exact: true }).click();
  await page.getByRole("dialog", { name: "Change language", exact: true }).getByRole("button", { name: "Confirm", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  // The static HTML may already say zh-CN before scripts commit the preference.
  await expect(page.getByRole("button", { name: "切换为英文", exact: true })).toBeVisible();
  await page.goto(chineseUrl);
  await expect(page.getByRole("button", { name: "切换为英文", exact: true })).toBeVisible();
}
