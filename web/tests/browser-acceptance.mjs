import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const session = { authenticated: true, user_id: "A".repeat(43), username: "admin", role: "admin", csrf_token: "A".repeat(43) };
const time = "2026-09-04T00:00:00Z", GIB = 2 ** 30;
function backupUser() {
  return { id: "018f1f4b-7a5d-7b5f-8d31-123456789abc", username: "backup", display_name: "验收备份账户", storage_path: "blobs/acceptance", quota_bytes: 123456789,
    used_bytes: 1024, pending_bytes: 512, device_count: 2, resource_count: 3, enabled: true, created_at: time, last_seen_at: time };
}
const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
const address = server.httpServer.address();
assert.ok(address && typeof address === "object");
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN",  viewport: { width: 360, height: 740 } });
      const page = await context.newPage(), errors = [], mutations = [];
      let users = [backupUser()], failOverview = false, failReset = true, failCreate = true, administratorCreated = false;
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v2/**", async route => {
        const request = route.request(), path = new URL(request.url()).pathname, method = request.method();
        if (path === "/api/v2/auth/session") return route.fulfill({ json: session });
        if (method !== "GET") {
          assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
          mutations.push({ path, method, body: request.postDataJSON() });
        }
        const failure = id => route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET database path", retryable: false, request_id: id } });
        if (path === "/api/v2/admin/overview") {
          if (failOverview) { failOverview = false; return failure("overview-failure-123"); }
          return route.fulfill({ json: { users, total_users: users.length, active_users: users.filter(user => user.enabled).length,
            unlimited_users: users.filter(user => user.quota_bytes === 0).length, used_bytes: 1024, pending_bytes: 512, quota_bytes: 123456789 } });
        }
        if (path === "/api/v2/admin/users" && method === "POST") {
          if (failCreate) { failCreate = false; return failure("create-failure-123"); }
          const { password, ...input } = request.postDataJSON();
          assert.equal(password, "test backup password"); assert.equal(input.quota_bytes, 1.25 * GIB);
          const created = { ...backupUser(), ...input, id: "018f1f4b-7a5d-7b5f-8d31-123456789abd", storage_path: "blobs/new-user" };
          users.push(created); return route.fulfill({ json: created });
        }
        if (path === `/api/v2/admin/users/${users[0].id}` && method === "PUT") {
          assert.equal(request.postDataJSON().quota_bytes, 123456789);
          assert.ok(!("password" in request.postDataJSON()));
          users[0] = { ...users[0], ...request.postDataJSON() }; return route.fulfill({ json: users[0] });
        }
        if (path === `/api/v2/admin/users/${users[0].id}/reset-password` && method === "POST") {
          assert.deepEqual(request.postDataJSON(), { password: "reset backup password" });
          if (failReset) { failReset = false; return failure("reset-failure-123"); }
          return route.fulfill({ status: 204 });
        }
        if (path === "/api/v2/platform/administrators") {
          if (method === "POST") { assert.deepEqual(request.postDataJSON(), { username: "secondary", password: "test admin password" }); administratorCreated = true; return route.fulfill({ status: 204 }); }
          const record = { administrator_id: session.user_id, username: "admin", active: true, created_at_micros: 1, updated_at_micros: 2, last_login_at_micros: null };
          return route.fulfill({ json: administratorCreated ? [record, { ...record, administrator_id: "B".repeat(43), username: "secondary" }] : [record] });
        }
        throw new Error(`Unexpected API request ${method} ${path}`);
      });
      await page.goto(`http://127.0.0.1:${address.port}/admin/`);
      await expect(page.getByRole("heading", { name: "备份总览", exact: true })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.getByRole("banner").locator('.sarmg-product-identity')).toHaveText("Media Backup");
      await expect(page).toHaveTitle("Media Backup");
      const statistics = page.getByRole("table", { name: "备份统计", exact: true });
      await expect(statistics.getByRole("columnheader")).toHaveText(["统计项", "当前值"]);
      await expect(statistics.getByRole("rowheader")).toHaveText(["启用 / 全部用户", "媒体已用", "上传预留空间", "已分配配额"]);
      await expect(statistics.getByRole("cell")).toHaveText(["1 / 1", "1.0 KiB", "512 B", "117.7 MiB"]);
      assert.equal(await statistics.locator("tbody tr").first().evaluate(row => getComputedStyle(row).display), "table-row");
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      assert.deepEqual((await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze()).violations, []);
      await expect(page.getByRole("banner").locator('small')).toHaveCount(0);
      await expect(page.getByRole("heading", { name: "验收备份账户", exact: true })).toBeVisible();
      assert.ok(await page.evaluate(async () => {
        const normal = await document.fonts.load('16px "Sarmg Maple"');
        const italic = await document.fonts.load('italic 16px "Sarmg Maple"');
        return normal.length > 0 && italic.length > 0 && [...normal, ...italic].every(font => font.status === "loaded");
      }));
      const provenance = JSON.parse(readFileSync(new URL("../fonts/provenance.json", import.meta.url), "utf8"));
      const names = Object.keys(provenance.assets).filter(name => name.endsWith(".woff2")).map(name => name.split("/").at(-1));
      for (const name of [...names, "MapleMono-OFL.txt", "CJK-LICENSE.txt"]) {
        const asset = await context.request.get(`http://127.0.0.1:${address.port}/admin/assets/${name}`);
        assert.equal(asset.status(), 200); assert.deepEqual(await asset.body(), readFileSync(new URL(`../dist/assets/${name}`, import.meta.url)));
      }
      failOverview = true;
      await page.getByRole("button", { name: "刷新", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("overview-failure-123");
      await expect(page.getByRole("heading", { name: "验收备份账户", exact: true })).toHaveCount(0);
      await expect(page.locator("body")).not.toContainText("SECRET");
      await page.getByRole("button", { name: "重试", exact: true }).click();
      await expect(page.getByRole("heading", { name: "验收备份账户", exact: true })).toBeVisible();
      await page.getByRole("button", { name: "备份用户", exact: true }).click();
      await expect(page.getByRole("complementary", { name: "备份用户实例" })).toBeVisible();
      await page.getByRole("button", { name: "新建备份用户", exact: true }).click();
      const create = page.getByRole("form", { name: "创建备份用户", exact: true });
      await create.getByLabel("名称", { exact: true }).fill("新建备份账户");
      await create.getByLabel("账号", { exact: true }).fill("new-backup");
      await create.getByLabel("密码", { exact: true }).fill("test backup password");
      await create.getByLabel("配额（GiB，0 表示不限）", { exact: true }).fill("1.25");
      await create.getByRole("button", { name: "创建备份用户", exact: true }).click();
      await expect(create.getByRole("alert")).toContainText("create-failure-123");
      await expect(create.getByLabel("密码", { exact: true })).toHaveValue("");
      await expect(create.getByLabel("密码", { exact: true })).toBeFocused();
      assert.equal(mutations.length, 1);
      await create.getByLabel("密码", { exact: true }).fill("test backup password");
      await create.getByRole("button", { name: "创建备份用户", exact: true }).click();
      await page.getByRole("button", { name: "选择实例 新建备份账户", exact: true }).click();
      await expect(page.getByRole("form", { name: "编辑备份用户 new-backup", exact: true })).toBeVisible();
      await page.getByRole("button", { name: "选择实例 验收备份账户", exact: true }).click();
      const edit = page.getByRole("form", { name: "编辑备份用户 backup", exact: true });
      await edit.getByLabel("名称", { exact: true }).fill("已更新备份账户");
      await edit.getByRole("button", { name: "保存备份用户", exact: true }).click();
      await expect(page.getByRole("heading", { name: "已更新备份账户", exact: true })).toBeVisible();
      assert.equal(mutations.filter(item => item.path.endsWith("/reset-password")).length, 0);
      await edit.getByRole("button", { name: "重设备份密码", exact: true }).click();
      const reset = page.getByRole("dialog", { name: "重设备份密码 · backup", exact: true });
      for (let i = 0; i < 6; i++) { await page.keyboard.press("Tab"); assert.ok(await reset.evaluate(element => element.contains(document.activeElement))); }
      await reset.getByLabel("新备份密码", { exact: true }).fill("reset backup password");
      await reset.getByRole("button", { name: "确认重设密码", exact: true }).click();
      await expect(reset.getByRole("alert")).toContainText("reset-failure-123");
      await expect(reset.getByLabel("新备份密码", { exact: true })).toHaveValue("");
      await expect(reset.getByLabel("新备份密码", { exact: true })).toBeFocused();
      await expect(reset).not.toContainText("SECRET");
      assert.equal(mutations.filter(item => item.path.endsWith("/reset-password")).length, 1);
      await reset.getByLabel("新备份密码", { exact: true }).fill("reset backup password");
      await reset.getByRole("button", { name: "确认重设密码", exact: true }).click();
      await expect(reset).toHaveCount(0);
      await edit.getByLabel("启用备份用户", { exact: true }).uncheck();
      await edit.getByRole("button", { name: "保存备份用户", exact: true }).click();
      const disable = page.getByRole("dialog", { name: "停用备份用户 backup？", exact: true });
      await expect(disable.getByRole("button", { name: "取消", exact: true })).toBeFocused();
      const beforeCancel = mutations.length;
      await page.keyboard.press("Escape"); await expect(disable).toHaveCount(0); assert.equal(mutations.length, beforeCancel);
      await edit.getByRole("button", { name: "保存备份用户", exact: true }).click();
      await disable.getByRole("button", { name: "确认", exact: true }).click();
      await expect(edit.getByLabel("启用备份用户", { exact: true })).not.toBeChecked();
      await expect.poll(() => users[0].enabled).toBe(false);
      for (const theme of ["light", "dark"]) {
        if (await page.locator("html").getAttribute("data-theme") !== theme) await page.getByRole("button", { name: /切换到.*模式/ }).click();
        assert.deepEqual((await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze()).violations, []);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      }
      await page.getByRole("button", { name: "平台管理员", exact: true }).click();
      await expect(page.getByRole("form", { name: "创建备份用户", exact: true })).toHaveCount(0);
      await page.getByRole("button", { name: "创建管理员", exact: true }).click();
      await page.getByLabel("用户名", { exact: true }).fill("secondary");
      await page.getByLabel("新密码", { exact: true }).fill("test admin password");
      await page.getByRole("button", { name: "保存管理员", exact: true }).click();
      await expect(page.getByRole("rowheader", { name: "secondary", exact: true })).toBeVisible();
      await checkWebLanguage(page, {"routes":[["overview","Overview"],["users","Backup users"],["administrators","Platform administrators"]],"names":["验收备份账户","新建备份账户","已更新备份账户"]});
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: Media overview/retry, backup account create/edit/disable/reset, separate platform administrators, font assets, modal focus and mobile WCAG AA passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
