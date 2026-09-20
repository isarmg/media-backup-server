import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const session = { authenticated: true, user_id: "A".repeat(43), username: "admin", role: "admin", csrf_token: "A".repeat(43) };
async function assertColumnContentAlignment(table) {
  const offsets = await table.evaluate(element => {
    const textStart = cell => {
      const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT); let text;
      while ((text = walker.nextNode()) && !text.textContent.trim()) {}
      if (!text) throw new Error("table cell has no visible text");
      const range = document.createRange(); range.selectNodeContents(text);
      return range.getBoundingClientRect().left;
    };
    const contentStart = cell => cell.firstElementChild?.getBoundingClientRect().left ?? textStart(cell);
    const headings = [...element.querySelectorAll("thead th")], values = [...element.querySelector("tbody tr").children];
    if (headings.length !== values.length) throw new Error("table column count mismatch");
    return headings.map((heading, index) => Math.abs(textStart(heading) - contentStart(values[index])));
  });
  assert.ok(offsets.every(offset => offset < 0.5), `column content offsets: ${JSON.stringify(offsets)}`);
}
const time = "2026-09-04T00:00:00Z", userId = "018f1f4b-7a5d-7b5f-8d31-123456789abc";
const instanceId = "018f1f4b-7a5d-7b5f-8d31-123456789abd", code = "m".repeat(32);
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
      let users = [backupUser()], failCreate = true, failRotate = true, failDelete = true;
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v2/**", async route => {
        const request = route.request(), path = new URL(request.url()).pathname, method = request.method();
        if (path === "/api/v2/auth/session") return route.fulfill({ json: session });
        if (method !== "GET") {
          assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
          mutations.push({ path, method });
        }
        if (path === "/api/v2/admin/overview") return route.fulfill({ json: {
          users, total_users: users.length,
          unlimited_users: users.filter(user => user.quota_bytes === 0).length,
          used_bytes: 1024, pending_bytes: 512, quota_bytes: users.reduce((sum, user) => sum + user.quota_bytes, 0),
        } });
        if (path === "/api/v2/admin/logs") return route.fulfill({ json: [{ sequence: 1, action: "backup.instance.create", entity_id: instanceId, occurred_at: time }] });
        if (path === "/api/v2/admin/instances" && method === "POST") {
          const input = request.postDataJSON();
          assert.deepEqual(input, {});
          if (failCreate) { failCreate = false; return route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET", retryable: false, request_id: "create-failure-123" } }); }
          const created = instance({ id: "018f1f4b-7a5d-7b5f-8d31-000000000001", name: "新实例", authorization_code: "n".repeat(32) });
          users.push(backupUser({ id: "018f1f4b-7a5d-7b5f-8d31-123456789abe", username: "instance-internal", display_name: "新实例",
            storage_path: "blobs/automatic", quota_bytes: 107374182400, instances: [created], used_bytes: 0, pending_bytes: 0, device_count: 1, resource_count: 0 }));
          return route.fulfill({ status: 201, json: created });
        }
        const rotateMatch = path.match(/^\/api\/v2\/admin\/instances\/([^/]+)\/authorization$/);
        if (rotateMatch && method === "PUT") {
          if (failRotate) { failRotate = false; return route.fulfill({ status: 503, json: { code: "service_unavailable", message: "SECRET rotate", retryable: true, request_id: "rotate-failure-123" } }); }
          const target = users.flatMap(user => user.instances).find(item => item.id === rotateMatch[1]);
          Object.assign(target, { authorization_code: "r".repeat(32), status: "pending" });
          return route.fulfill({ json: target });
        }
        const removeMatch = path.match(/^\/api\/v2\/admin\/instances\/([^/]+)$/);
        if (removeMatch && method === "DELETE") {
          const target = users.flatMap(user => user.instances).find(item => item.id === removeMatch[1]);
          if (failDelete && (target?.status === "cancelled" || target?.status === "revoked")) {
            failDelete = false;
            return route.fulfill({ status: 409, json: { code: "conflict", message: "SECRET backup path", retryable: false, request_id: "delete-failure-123" } });
          }
          for (const [userIndex, user] of users.entries()) {
            const current = user.instances.find(item => item.id === removeMatch[1]);
            if (!current) continue;
            if (current.status === "cancelled" || current.status === "revoked") users.splice(userIndex, 1);
            else current.status = current.status === "pending" ? "cancelled" : "revoked";
          }
          return route.fulfill({ status: 204 });
        }
        throw new Error(`Unexpected API request ${method} ${path}`);
      });

      await page.goto(`http://127.0.0.1:${address.port}/admin/`);
      await expect(page.getByRole("button", { name: "实例列表", exact: true })).toHaveAttribute("aria-pressed", "true");
      const statistics = page.getByRole("table", { name: "实例统计" });
      await expect(statistics.getByRole("columnheader")).toHaveText(["统计项", "总数 / 在线"]);
      await expect(statistics).not.toContainText("待配对实例");
      await expect(statistics).not.toContainText("启用 / 全部实例");
      await expect(statistics.getByRole("row").nth(1).locator("th, td")).toHaveText(["总数", "1 / 0"]);
      const instanceTable = page.getByRole("table", { name: "实例列表" });
      await expect(instanceTable.getByRole("link", { name: "验收备份账户" })).toBeVisible();
      await expect(instanceTable.getByRole("columnheader")).toHaveText(["实例", "备份状态", "配对状态", "在线状态", "客户端 / 平台", "最后在线", "已用容量 / 配额", "删除"]);
      assert.ok((await instanceTable.locator("th, td").evaluateAll(elements => elements.map(element => getComputedStyle(element).textAlign))).every(value => value === "left"));
      await assertColumnContentAlignment(instanceTable);
      assert.ok((await instanceTable.locator(".sarmg-actions").evaluateAll(elements => elements.map(element => getComputedStyle(element).justifyContent))).every(value => value === "flex-start"));
      await expect(page.getByRole("complementary")).toHaveCount(0);
      assert.deepEqual((await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa"]).analyze()).violations, []);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));

      await page.getByRole("button", { name: "新建实例", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("create-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET");
      await page.getByRole("button", { name: "新建实例", exact: true }).click();
      await expect(page.getByRole("button", { name: "关闭通知", exact: true })).toBeVisible();
      await expect(page.getByRole("button", { name: "关闭通知", exact: true })).toHaveCount(0, { timeout: 7_000 });
      await expect(page.getByRole("link", { name: "新实例", exact: true })).toBeVisible();
      const createdRow = instanceTable.locator("tbody tr").filter({ has: page.getByRole("link", { name: "新实例", exact: true }) });
      await createdRow.getByRole("button", { name: "删除", exact: true }).click();
      await createdRow.getByRole("button", { name: "取消", exact: true }).click();
      await expect(createdRow.getByRole("button", { name: "确认删除", exact: true })).toHaveCount(0);

      await page.getByRole("link", { name: "验收备份账户", exact: true }).click();
      await expect(page.getByRole("button", { name: "详细信息", exact: true })).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByRole("form", { name: "编辑备份实例 验收备份账户", exact: true }).getByLabel("密码")).toHaveCount(0);
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.getByText(code, { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "更换密码", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("rotate-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET rotate");
      await page.getByRole("button", { name: "更换密码", exact: true }).click();
      await expect(page.getByText("r".repeat(32), { exact: true })).toBeVisible();
      await page.getByRole("button", { name: "取消配对", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("cancelled", { exact: true })).toBeVisible();
      failDelete = true;
      await page.getByRole("button", { name: "实例列表", exact: true }).click();
      const originalRow = instanceTable.locator("tbody tr").filter({ has: page.getByRole("link", { name: "验收备份账户", exact: true }) });
      await originalRow.getByRole("button", { name: "删除", exact: true }).click();
      await originalRow.getByRole("button", { name: "确认删除", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("delete-failure-123");
      await expect(page.locator("body")).not.toContainText("SECRET backup path");
      await originalRow.getByRole("button", { name: "确认删除", exact: true }).click();
      await expect(page.getByRole("link", { name: "验收备份账户", exact: true })).toHaveCount(0);

      await page.getByRole("button", { name: "日志", exact: true }).click();
      await expect(page.getByText("backup.instance.create", { exact: true })).toBeVisible();
      await page.goto(`http://127.0.0.1:${address.port}/admin/#details/018f1f4b-7a5d-7b5f-8d31-123456789abe`);
      await expect(page.getByRole("button", { name: "详细信息", exact: true })).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByRole("form", { name: "编辑备份实例 新实例", exact: true })).toBeVisible();
      await checkWebLanguage(page, { routes: [["details/018f1f4b-7a5d-7b5f-8d31-123456789abe", "Details"], ["instances", "Instance list"], ["logs", "Logs"]], names: ["新实例"] });
      assert.ok(mutations.some(item => item.path.endsWith("/authorization")));
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: atomic instance creation, details/logs, authorization rotation, cancel/delete, direct language switch and WCAG passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
