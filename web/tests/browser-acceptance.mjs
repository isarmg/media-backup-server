import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const session = { authenticated: true, user_id: "A".repeat(43), username: "admin", role: "admin", csrf_token: "A".repeat(43) };
const time = "2026-09-04T00:00:00Z", userId = "018f1f4b-7a5d-7b5f-8d31-123456789abc";
const instanceId = "018f1f4b-7a5d-7b5f-8d31-123456789abd", code = "m".repeat(43);
function instance(overrides = {}) {
  return { id: instanceId, name: "验收手机", platform: "android", status: "pending", online: false, authorization_code: code, created_at: time, last_seen_at: "", ...overrides };
}
function backupUser(overrides = {}) {
  return { id: userId, username: "backup", display_name: "验收备份账户", storage_path: "blobs/acceptance", quota_bytes: 123456789,
    used_bytes: 1024, pending_bytes: 512, device_count: 1, resource_count: 3, enabled: true, created_at: time, last_seen_at: time,
    instances: [instance()], ...overrides };
}

const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
const address = server.httpServer.address();
assert.ok(address && typeof address === "object");
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN", viewport: { width: 390, height: 844 } });
      const page = await context.newPage(), errors = [], mutations = [];
      let users = [backupUser()], failCreate = true;
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v2/**", async route => {
        const request = route.request(), path = new URL(request.url()).pathname, method = request.method();
        if (path === "/api/v2/auth/session") return route.fulfill({ json: session });
        if (method !== "GET") {
          assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
          mutations.push({ path, method });
        }
        if (path === "/api/v2/admin/overview") return route.fulfill({ json: {
          users, total_users: users.length, active_users: users.filter(user => user.enabled).length,
          unlimited_users: users.filter(user => user.quota_bytes === 0).length,
          used_bytes: 1024, pending_bytes: 512, quota_bytes: users.reduce((sum, user) => sum + user.quota_bytes, 0),
        } });
        if (path === "/api/v2/admin/logs") return route.fulfill({ json: [{ sequence: 1, action: "backup.instance.create", entity_id: instanceId, occurred_at: time }] });
        if (path === "/api/v2/admin/instances" && method === "POST") {
          const input = request.postDataJSON();
          assert.deepEqual(Object.keys(input), ["name"]);
          if (failCreate) { failCreate = false; return route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET", retryable: false, request_id: "create-failure-123" } }); }
          const created = instance({ id: "018f1f4b-7a5d-7b5f-8d31-000000000001", name: input.name, authorization_code: "n".repeat(43) });
          users.push(backupUser({ id: "018f1f4b-7a5d-7b5f-8d31-123456789abe", username: "instance-internal", display_name: input.name,
            storage_path: "blobs/automatic", quota_bytes: 107374182400, instances: [created], used_bytes: 0, pending_bytes: 0, device_count: 1, resource_count: 0 }));
          return route.fulfill({ status: 201, json: created });
        }
        const rotateMatch = path.match(/^\/api\/v2\/admin\/instances\/([^/]+)\/authorization$/);
        if (rotateMatch && method === "PUT") {
          const target = users.flatMap(user => user.instances).find(item => item.id === rotateMatch[1]);
          Object.assign(target, { authorization_code: "r".repeat(43), status: "pending" });
          return route.fulfill({ json: target });
        }
        const removeMatch = path.match(/^\/api\/v2\/admin\/instances\/([^/]+)$/);
        if (removeMatch && method === "DELETE") {
          for (const [userIndex, user] of users.entries()) {
            const target = user.instances.find(item => item.id === removeMatch[1]);
            if (!target) continue;
            if (target.status === "cancelled" || target.status === "revoked") users.splice(userIndex, 1);
            else target.status = target.status === "pending" ? "cancelled" : "revoked";
          }
          return route.fulfill({ status: 204 });
        }
        throw new Error(`Unexpected API request ${method} ${path}`);
      });

      await page.goto(`http://127.0.0.1:${address.port}/admin/`);
      await expect(page.getByRole("button", { name: "实例列表", exact: true })).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByRole("table", { name: "实例统计" })).toBeVisible();
      await expect(page.getByRole("table", { name: "实例列表" }).getByRole("link", { name: "验收备份账户" })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      assert.deepEqual((await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze()).violations, []);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));

      await page.getByRole("button", { name: "新建实例", exact: true }).click();
      const create = page.getByRole("form", { name: "创建备份实例", exact: true });
      await expect(create.getByLabel("密码")).toHaveCount(0);
      await expect(create.getByLabel("账号", { exact: true })).toHaveCount(0);
      await create.getByLabel("实例名称", { exact: true }).fill("新建备份实例");
      await expect(create.getByLabel("存储路径（留空自动分配）", { exact: true })).toHaveCount(0);
      await expect(create.getByLabel("配额（GiB，0 表示不限）", { exact: true })).toHaveCount(0);
      await create.getByRole("button", { name: "创建实例", exact: true }).click();
      await expect(create.getByRole("alert")).toContainText("create-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET");
      await create.getByRole("button", { name: "创建实例", exact: true }).click();
      await expect(create).toHaveCount(0);
      await expect(page.getByRole("textbox", { name: "授权码", exact: true })).toHaveValue("n".repeat(43));
      await page.getByRole("button", { name: "已保存，关闭", exact: true }).click();
      await expect(page.getByRole("link", { name: "新建备份实例", exact: true })).toBeVisible();

      await page.getByRole("link", { name: "验收备份账户", exact: true }).click();
      await expect(page.getByRole("button", { name: "详细信息", exact: true })).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByRole("form", { name: "编辑备份实例 验收备份账户", exact: true }).getByLabel("密码")).toHaveCount(0);
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.getByText(code, { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "更换授权码", exact: true }).click();
      await expect(page.getByText("r".repeat(43), { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "取消配对", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("cancelled", { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "删除实例", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("请选择一个备份实例。", { exact: true })).toBeVisible();

      await page.getByRole("button", { name: "日志", exact: true }).click();
      await expect(page.getByText("backup.instance.create", { exact: true })).toBeVisible();
      await page.goto(`http://127.0.0.1:${address.port}/admin/#details/018f1f4b-7a5d-7b5f-8d31-123456789abe`);
      await expect(page.getByRole("form", { name: "编辑备份实例 新建备份实例", exact: true })).toBeVisible();
      await checkWebLanguage(page, { routes: [["details/018f1f4b-7a5d-7b5f-8d31-123456789abe", "Details"], ["instances", "Instance list"], ["logs", "Logs"]], names: ["新建备份实例"] });
      assert.ok(mutations.some(item => item.path.endsWith("/authorization")));
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: atomic instance creation, details/logs, authorization rotation, cancel/delete, direct language switch and WCAG passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
