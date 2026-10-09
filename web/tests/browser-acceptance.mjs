import { checkAccountPage } from "./account-page.mjs";
import { rangeEditor, checkDateRangeValidation } from "./date-range.mjs";
import { checkWebLanguage } from "./language.mjs";
import assert from "node:assert/strict";
import { chromium, firefox, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { preview } from "vite";

const session = { authenticated: true, user_id: "A".repeat(43), username: "admin", role: "admin", csrf_token: "A".repeat(43) };
async function assertLogControlsLayout(page) {
  const viewport = page.viewportSize();
  for (const width of [1838, 360]) {
    await page.setViewportSize({ width, height: viewport.height });
    const layout = await page.locator(".xcss-log-date-controls").evaluate(element => {
      const label = element.querySelector("label").getBoundingClientRect();
      const button = element.querySelector("button:last-child").getBoundingClientRect();
      const input = element.querySelector(".xcss-date-range-fields").getBoundingClientRect();
      const controls = element.getBoundingClientRect();
      const textTop = node => { const range = document.createRange(); range.selectNodeContents(node); return range.getBoundingClientRect().top; };
      const textOffset = textTop(element.querySelector("button:last-child")) - textTop(element.querySelector("label"));
      return { textOffset, rowOffset: button.top - label.top, leftOffset: input.left - label.left,
        rightOffset: input.right - button.right, widthOffset: controls.width - input.width,
        rowGap: input.top - Math.max(label.bottom, button.bottom), right: controls.right };
    });
    const positions = await page.getByRole("region", { name: "备份日志", exact: true }).evaluate(element => {
      const name = element.querySelector(".xcss-content-panel > p").getBoundingClientRect();
      const label = element.querySelector(".xcss-log-date-controls > label").getBoundingClientRect();
      const table = element.querySelector(".xcss-table-scroll").getBoundingClientRect();
      const pager = element.querySelector('nav[aria-label="日志分页"]').getBoundingClientRect();
      return { nameLeft: name.left-label.left, nameAbove: name.bottom<label.top, pagerBelow: pager.top>=table.bottom, rightAligned: Math.abs(pager.right-table.right)<1 };
    });
    assert.ok(Math.abs(positions.nameLeft)<1 && positions.nameAbove && positions.pagerBelow && positions.rightAligned, JSON.stringify(positions));
    assert.ok(Math.abs(layout.textOffset) <= 1 && Math.abs(layout.rowOffset) < 1 && Math.abs(layout.leftOffset) < 1 && Math.abs(layout.rightOffset) < 1 && Math.abs(layout.widthOffset) < 1, JSON.stringify(layout));
    assert.ok(layout.rowGap >= 7 && layout.rowGap <= 9 && layout.right <= width, JSON.stringify(layout));
  }
  await page.setViewportSize(viewport);
}
async function assertColumnContentAlignment(table) {
  const offsets = await table.evaluate(element => {
    const textStart = cell => {
      const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT); let text;
      while ((text = walker.nextNode()) && !text.textContent.trim()) {}
      if (!text) throw new Error("table cell has no visible text");
      const range = document.createRange(); range.selectNodeContents(text);
      return range.getBoundingClientRect().left;
    };
    const contentStart = cell => textStart(cell);
    const headings = [...element.querySelectorAll("thead th")], values = [...element.querySelector("tbody tr").children];
    if (headings.length !== values.length) throw new Error("table column count mismatch");
    return headings.map((heading, index) => Math.abs(textStart(heading) - contentStart(values[index])));
  });
  assert.ok(offsets.every(offset => offset < 0.5), `column content offsets: ${JSON.stringify(offsets)}`);
}
const time = "2026-09-04T00:00:00Z", userId = "018f1f4b-7a5d-7b5f-8d31-123456789abc";
const instanceId = "018f1f4b-7a5d-7b5f-8d31-123456789abd", code = "m".repeat(36);
const serverDate = "2042-07-06", previousDate = "2042-07-05";
const logBudgetPadding = "x".repeat(128 * 1024);
function instance(overrides = {}) {
  return { id: instanceId, name: "验收手机", platform: "android", status: "pending", online: false, authorization_code: code, created_at: time, last_seen_at: "", ...overrides };
}
function backupUser(overrides = {}) {
  return { id: userId, username: "backup", display_name: "验收备份账户", storage_path: "blobs/acceptance", quota_bytes: 123456789,
    used_bytes: 1024, pending_bytes: 512, device_count: 1, resource_count: 3, created_at: time, last_seen_at: time,
    instances: [instance()], ...overrides };
}

const server = await preview({ preview: { host: "127.0.0.1", port: 0, strictPort: true } });
const address = server.httpServer.address();
assert.ok(address && typeof address === "object");
try {
  for (const engine of [chromium, firefox]) {
    const browser = await engine.launch();
    try {
      const context = await browser.newContext({ locale: "zh-CN", timezoneId: "America/Los_Angeles", viewport: { width: 390, height: 844 } });
      const page = await context.newPage(), errors = [], mutations = [];
      const logQueries = [], overviewQueries = [], detailQueries = [];
      let users = [backupUser()], failCreate = true, failRotate = true, failDelete = true, oversizeNextLog = false;
      const expectedQuotaUpdates = [123456789, 107374182];
      let holdRotate = false, releaseRotate;
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/api/v1/**", async route => {
        const request = route.request(), path = new URL(request.url()).pathname, method = request.method();
        if (path === "/api/v1/auth/session") return route.fulfill({ json: session });
        if (method !== "GET") {
          assert.equal(request.headers()["x-csrf-token"], session.csrf_token);
          mutations.push({ path, method });
        }
        if (path === "/api/v1/admin/overview") {
          const cursor = new URL(request.url()).searchParams.get("cursor"); overviewQueries.push(cursor);
          const fold = value => value.replace(/[A-Z]/g, letter => letter.toLowerCase());
          const compare = (left, right) => {
            for (const [a,b] of [[fold(left.display_name),fold(right.display_name)], [left.display_name,right.display_name], [left.id,right.id]]) {
              if (a !== b) return a < b ? -1 : 1;
            }
            return 0;
          };
          const all = users.toSorted(compare);
          const anchor = cursor === null ? null : JSON.parse(Buffer.from(cursor, "base64url").toString());
          let eligible = anchor === null ? all : all.filter(user => anchor.before ? compare(user,{display_name:anchor.name,id:anchor.id})<0 : compare(user,{display_name:anchor.name,id:anchor.id})>0);
          if (anchor?.before) eligible = eligible.toReversed();
          let pageUsers = eligible.slice(0,50); if (anchor?.before) pageUsers = pageUsers.toReversed();
          const encode = (user,before) => Buffer.from(JSON.stringify({name:user.display_name,id:user.id,before})).toString("base64url");
          return route.fulfill({json:{users:pageUsers,
            previous_cursor:pageUsers.length && all.some(user => compare(user,pageUsers[0])<0) ? encode(pageUsers[0],true) : null,
            next_cursor:pageUsers.length && all.some(user => compare(user,pageUsers.at(-1))>0) ? encode(pageUsers.at(-1),false) : null,
            total_users: users.length, online_users:users.filter(user=>user.instances.some(instance=>instance.status==="paired"&&instance.online)).length,
            unlimited_users:users.filter(user=>user.quota_bytes===0).length,
            used_bytes:users.reduce((sum,user)=>sum+user.used_bytes,0),
            pending_bytes:users.reduce((sum,user)=>sum+user.pending_bytes,0),
            quota_bytes:users.reduce((sum,user)=>sum+user.quota_bytes,0),
          }});
        }
        if (/^\/api\/v1\/admin\/users\/[^/]+$/.test(path) && method === "GET") {
          const id = path.split("/").at(-1); detailQueries.push(id);
          const user = users.find(user=>user.id===id);
          return user ? route.fulfill({json:user}) : route.fulfill({status:404,json:{code:"platform.not_found",message:"not found",retryable:false,request_id:"fixture-not-found"}});
        }
        if (path === "/api/v1/admin/logs") {
          const params = new URL(request.url()).searchParams;
          const date = params.get("date");
          const startDate = params.get("start_date"), endDate = params.get("end_date");
          const scope = params.get("instance_id") ?? "all";
          const cursor = params.get("cursor");
          logQueries.push(date);
          assert.ok(date === null || date === serverDate || date === previousDate);
          assert.ok(scope === "all" || scope === instanceId || users.some(user => user.instances.some(instance => instance.id === scope)));
          const actualDate = date ?? startDate ?? serverDate;
          const cursorDate = endDate ? `${actualDate}/${endDate}` : actualDate;
          const allLogs = (actualDate === previousDate && endDate === null
            ? [{ sequence: 1, action: "backup.instance.rotate", entity_id: instanceId, occurred_at: `${previousDate} 23:59:59 +08:00` }]
            : Array.from({ length: 225 }, (_, index) => ({ sequence: 226 - index, action: `backup.instance.create.${224 - index}`, entity_id: index < 5 ? "018f1f4b-7a5d-7b5f-8d31-000000000001" : instanceId, occurred_at: `${serverDate} 00:00:00 +08:00` }))).filter(log => scope === "all" || log.entity_id === scope);
          let eligible = allLogs;
          if (cursor !== null) {
            const parts = cursor.split(":");
            assert.equal(parts[0], cursorDate); assert.equal(parts[1], scope);
            eligible = allLogs.filter(log => parts[2] === "older" ? log.sequence < Number(parts[3]) : log.sequence > Number(parts[3]));
            if (parts[2] === "newer") eligible = eligible.toReversed();
          }
          let logs = eligible.slice(0, 50);
          if (cursor?.split(":")[2] === "newer") logs = logs.toReversed();
          const previous_cursor = logs.length && allLogs.some(log => log.sequence > logs[0].sequence) ? `${cursorDate}:${scope}:newer:${logs[0].sequence}` : null;
          const next_cursor = logs.length && allLogs.some(log => log.sequence < logs.at(-1).sequence) ? `${cursorDate}:${scope}:older:${logs.at(-1).sequence}` : null;
          const padding = oversizeNextLog ? "x".repeat(1024 * 1024 + 1) : logBudgetPadding;
          oversizeNextLog = false;
          return route.fulfill({json: { date: actualDate, ...(endDate ? {end_date:endDate} : {}), instance_id: scope === "all" ? null : scope, logs, previous_cursor, next_cursor, transport_budget_fixture: padding }});
        }

        if (path === `/api/v1/admin/users/${userId}` && method === "PUT") {
          const input = request.postDataJSON();
          assert.equal(input.quota_bytes, expectedQuotaUpdates.shift());
          Object.assign(users[0], input);
          return route.fulfill({ json: users[0] });
        }
        if (path === "/api/v1/admin/instances" && method === "POST") {
          const input = request.postDataJSON();
          assert.deepEqual(input, { name: "新实例" });
          if (failCreate) { failCreate = false; return route.fulfill({ status: 500, json: { code: "platform.internal", message: "SECRET", retryable: false, request_id: "create-failure-123" } }); }
          const created = instance({ id: "018f1f4b-7a5d-7b5f-8d31-000000000001", name: "新实例", authorization_code: "n".repeat(36) });
          users.push(backupUser({ id: "018f1f4b-7a5d-7b5f-8d31-123456789abe", username: "instance-internal", display_name: "新实例",
            storage_path: "blobs/automatic", quota_bytes: 107374182400, instances: [created], used_bytes: 0, pending_bytes: 0, device_count: 1, resource_count: 0 }));
          return route.fulfill({ status: 201, json: created });
        }
        const rotateMatch = path.match(/^\/api\/v1\/admin\/instances\/([^/]+)\/authorization$/);
        if (rotateMatch && method === "PUT") {
          if (failRotate) { failRotate = false; return route.fulfill({ status: 503, json: { code: "service_unavailable", message: "SECRET rotate", retryable: true, request_id: "rotate-failure-123" } }); }
          if (holdRotate) await new Promise(resolve => { releaseRotate = resolve; });
          const target = users.flatMap(user => user.instances).find(item => item.id === rotateMatch[1]);
          Object.assign(target, { authorization_code: "r".repeat(36), status: "pending" });
          return route.fulfill({ json: target });
        }
        const removeMatch = path.match(/^\/api\/v1\/admin\/instances\/([^/]+)$/);
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
            else { current.status = current.status === "pending" ? "cancelled" : "revoked"; current.online=false; }
          }
          return route.fulfill({ status: 204 });
        }
        throw new Error(`Unexpected API request ${method} ${path}`);
      });

      await page.goto(`http://127.0.0.1:${address.port}/admin/`);
      await expect(page.getByRole("button", { name: "实例列表", exact: true })).toHaveAttribute("aria-pressed", "true");
      const statistics = page.getByRole("table", { name: "实例统计" });
      await expect(page.locator("h2").filter({ hasText: /^(实例|实例列表|统计)$/ })).toHaveCount(0);
      await page.setViewportSize({ width: 1838, height: 900 });
      const statisticPositions = await statistics.locator("tbody tr").evaluateAll(rows => rows.map(row => ({ label: row.querySelector("th").getBoundingClientRect().top, value: row.querySelector("td").getBoundingClientRect().top })));
      assert.equal(statisticPositions.length, 4);
      assert.ok(statisticPositions.every(row => Math.abs(row.label - statisticPositions[0].label) < 1 && Math.abs(row.value - row.label) < 1), JSON.stringify(statisticPositions));
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(statistics.getByRole("columnheader")).toHaveText(["统计项", "总数 / 在线"]);
      await expect(statistics).not.toContainText("待配对实例");
      await expect(statistics).not.toContainText("启用 / 全部实例");
      await expect(statistics.getByRole("row").nth(1).locator("th, td")).toHaveText(["总数", "1 / 0"]);
      const instanceTable = page.getByRole("table", { name: "实例列表" });
      await expect(instanceTable.getByRole("link", { name: "验收备份账户" })).toBeVisible();
      await checkAccountPage(page);
      const instanceNameStyle = await instanceTable.getByRole("link", { name: "验收备份账户" }).evaluate(element => ({ color: getComputedStyle(element).color, parentColor: getComputedStyle(element.parentElement).color, decoration: getComputedStyle(element).textDecorationLine }));
      assert.equal(instanceNameStyle.color, instanceNameStyle.parentColor);
      assert.equal(instanceNameStyle.decoration, "none");
      await expect(instanceTable.getByRole("columnheader")).toHaveText(["实例", "配对状态", "在线状态", "操作系统/架构", "最后在线", "已用容量 / 配额", "删除"]);
      await expect(instanceTable).not.toContainText(instanceId);
      await expect(instanceTable).not.toContainText(code);
      assert.ok((await instanceTable.locator("th, td").evaluateAll(elements => elements.map(element => getComputedStyle(element).textAlign))).every(value => value === "left"));
      await assertColumnContentAlignment(instanceTable);
      assert.ok((await instanceTable.locator(".xcss-actions").evaluateAll(elements => elements.map(element => getComputedStyle(element).justifyContent))).every(value => value === "flex-start"));
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
      await expect(page.getByRole("link", { name: "返回实例列表", exact: true })).toHaveCount(0);
      const pairingDetails = page.getByRole("region", { name: "配对账户信息" });
      await expect(pairingDetails.getByRole("heading", { name: "验收备份账户", exact: true })).toBeVisible();
      await expect(pairingDetails.getByText("实例名称", { exact: true })).toBeVisible();
      await expect(pairingDetails.getByText("实例 ID", { exact: true })).toBeVisible();
      await expect(pairingDetails).toContainText(instanceId);
      await expect(pairingDetails).toContainText(code);
      await expect(page.getByRole("region", { name: "实例设置" })).toBeVisible();
      await expect(page.getByRole("region", { name: "备份状态", exact: true })).toBeVisible();
      await expect(page.getByRole("region", { name: "实例操作", exact: true })).toBeVisible();
      await expect(page.getByRole("form", { name: "编辑备份实例 验收备份账户", exact: true }).getByLabel("密码")).toHaveCount(0);
      await expect(pairingDetails.getByText("密码", { exact: true })).toBeVisible();
      await expect(page.getByRole("complementary")).toHaveCount(0);
      await expect(page.getByText(code, { exact: true })).toBeVisible();
      const editForm = page.getByRole("form", { name: "编辑备份实例 验收备份账户", exact: true });
      await editForm.getByRole("button", { name: "保存设置" }).click();
      await expect.poll(() => expectedQuotaUpdates.length).toBe(1);
      await editForm.getByLabel("配额（GiB，0 表示不限）").fill("0.1");
      await editForm.getByRole("button", { name: "保存设置" }).click();
      await expect.poll(() => users[0].quota_bytes).toBe(107374182);
      const rotationCount = () => mutations.filter(item => item.path.endsWith("/authorization")).length;
      await page.getByRole("button", { name: "更换密码", exact: true }).click();
      const rotationDialog = page.getByRole("dialog", { name: "更换密码", exact: true });
      await expect(rotationDialog).toContainText("立即撤销当前客户端凭据");
      assert.equal(rotationCount(), 0);
      await rotationDialog.getByRole("button", { name: "取消", exact: true }).click();
      assert.equal(rotationCount(), 0);
      await page.getByRole("button", { name: "更换密码", exact: true }).click();
      await rotationDialog.getByRole("button", { name: "确认", exact: true }).click();
      await expect(rotationDialog.getByRole("alert")).toContainText("rotate-failure-123");
      await expect(rotationDialog).toContainText("核对密码后再操作");
      await expect(page.locator("body")).not.toContainText("SECRET rotate");
      await rotationDialog.getByRole("button", { name: "取消", exact: true }).click();
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect(page.getByText(code, { exact: true })).toBeVisible();
      holdRotate = true;
      await page.getByRole("button", { name: "更换密码", exact: true }).click();
      const confirmRotation = rotationDialog.getByRole("button", { name: "确认", exact: true });
      await confirmRotation.evaluate(button => { button.click(); button.click(); });
      await expect.poll(() => typeof releaseRotate).toBe("function");
      await expect(rotationDialog.getByRole("button", { name: "正在处理…", exact: true })).toBeDisabled();
      assert.equal(rotationCount(), 2, "duplicate confirmations must not dispatch another rotation");
      await page.keyboard.press("Escape");
      await expect(rotationDialog).toBeVisible();
      holdRotate = false;
      releaseRotate();
      await expect(page.getByText("r".repeat(36), { exact: true })).toBeVisible();
      assert.equal(rotationCount(), 2);
      await page.getByRole("button", { name: "取消配对", exact: true }).click();
      await page.getByRole("button", { name: "确认", exact: true }).click();
      await expect(page.getByText("已取消", { exact: true })).toBeVisible();
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
      const logDate = rangeEditor(page, "media-log-date");
      await logDate.expectValue(serverDate);
      await expect(page.getByRole("heading", { name: "日志", exact: true })).toHaveCount(0);
      await expect(page.getByText("按服务器日期显示所选日期的全部日志。", { exact: true })).toHaveCount(0);
      await assertLogControlsLayout(page);
      const logRows = page.getByRole("table").locator("tbody tr");
      await expect(logRows).toHaveCount(50);
      await expect(page.getByText("backup.instance.create.224", { exact: true })).toBeVisible();
      assert.ok(logQueries.every(date => date === null), "initial date comes from the server response");
      const actions = [];
      for (let index = 0; index < 5; index++) {
        await expect(logRows).toHaveCount(index === 4 ? 25 : 50);
        await expect(logRows.first()).toContainText(`backup.instance.create.${224 - index * 50}`);
        actions.push(...await logRows.locator("td:nth-child(2)").allTextContents());
        if (index < 4) await page.getByRole("button", {name: "下一页", exact: true}).click();
      }
      assert.equal(actions.length, 225); assert.equal(new Set(actions).size, 225, "same-time pages must have no duplicate or missing events");
      await expect(page.getByText("backup.instance.create.0", {exact:true})).toBeVisible();
      await expect(page.getByRole("button", {name:"下一页", exact:true})).toBeDisabled();
      await page.getByRole("button", {name:"上一页", exact:true}).click();
      await expect(logRows.first()).toContainText("backup.instance.create.74");
      await expect(logRows).toHaveCount(50);
      await page.getByRole("button", {name:"首页", exact:true}).click();
      await expect(logRows.first()).toContainText("backup.instance.create.224");
      await expect(page.getByRole("button", {name:"上一页", exact:true})).toBeDisabled();
      oversizeNextLog = true;
      await page.getByRole("button", {name:"刷新日志", exact:true}).click();
      await expect(page.getByText("日志暂不可用，请重试。", {exact:true})).toBeVisible();
      await expect(page.getByRole("table")).toHaveCount(0);
      await page.getByRole("button", {name:"重试", exact:true}).click();
      await expect(logRows).toHaveCount(50);
      await checkDateRangeValidation(logDate, () => logQueries.length, previousDate, serverDate);
      await expect(logRows).toHaveCount(50);
      await logDate.fill(previousDate);
      await expect(page.getByText("backup.instance.rotate", { exact: true })).toBeVisible();
      await expect(page.getByRole("table").locator("tbody tr")).toHaveCount(1);
      assert.equal(logQueries.at(-1), previousDate);
      await page.getByRole("button", { name: "刷新日志", exact: true }).click();
      await expect.poll(() => logQueries.filter(date => date === previousDate).length).toBe(2);
      await page.getByRole("group", { name: "全局操作" }).getByRole("button", { name: "刷新", exact: true }).click();
      await expect.poll(() => logQueries.filter(date => date === previousDate).length).toBe(3);
      await logDate.expectValue(previousDate);
      await logDate.fill("");
      await expect(page.getByRole("region", { name: "备份日志", exact: true })).toContainText("请输入有效的年月日。");
      await expect(page.getByRole("button", { name: "刷新日志", exact: true })).toBeDisabled();
      await expect(page.getByRole("region", { name: "备份日志", exact: true }).getByRole("alert")).toHaveCount(1);
      await page.goto(`http://127.0.0.1:${address.port}/admin/#details/018f1f4b-7a5d-7b5f-8d31-123456789abe`);
      await expect(page.getByRole("button", { name: "详细信息", exact: true })).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByRole("form", { name: "编辑备份实例 新实例", exact: true })).toBeVisible();
      await page.getByRole("button", {name: "日志", exact:true}).click();
      await expect(page.getByText("实例：新实例", {exact:true})).toBeVisible();
      await expect(page.getByRole("table").locator("tbody tr")).toHaveCount(5);
      await assertLogControlsLayout(page);
      await expect(page.getByRole("table")).not.toContainText(instanceId);
      await page.getByRole("button", {name:"全部实例", exact:true}).click();
      await expect(page.getByText("实例：全部实例", {exact:true})).toBeVisible();
      await page.goto(`http://127.0.0.1:${address.port}/admin/#details/018f1f4b-7a5d-7b5f-8d31-123456789abe`);
      await expect(page.getByRole("button", {name:"详细信息", exact:true})).toBeEnabled();
      await checkWebLanguage(page, { routes: [["details/018f1f4b-7a5d-7b5f-8d31-123456789abe", "Details"], ["instances", "Instance list"], ["logs", "Logs"]], names: ["新实例"] });
      // More than two pages with identical names must retain stable UUID order.
      users = Array.from({length:125}, (_,index)=>backupUser({
        id:`018f1f4b-7a5d-7b5f-8d31-${String(index+100000000000).padStart(12,"0")}`,
        username:`bulk-${index}`,display_name:"同名实例",used_bytes:1,pending_bytes:0,quota_bytes:1073741824,
        instances:[instance({id:`018f1f4b-7a5d-7b5f-8d31-${String(index+200000000000).padStart(12,"0")}`,name:"同名实例",status:"paired",online:index%2===0})],
      }));
      await page.goto(`http://127.0.0.1:${address.port}/admin/#instances`);
      const rows = instanceTable.locator("tbody tr"), navigation = page.getByRole("navigation",{name:"实例分页",exact:true});
      const visited = [];
      for(let index=0;index<3;index++) {
        await expect(rows).toHaveCount(index===2?25:50);
        await expect(rows.first().getByRole("link")).toHaveAttribute("href",`#details/${users[index*50].id}`);
        await expect(statistics.getByRole("row").nth(1)).toContainText("125 / 63");
        await expect(statistics.getByRole("row").nth(2)).toContainText("125 B");
        visited.push(...await rows.getByRole("link").evaluateAll(links=>links.map(link=>link.getAttribute("href"))));
        if(index<2)await navigation.getByRole("button",{name:"下一页",exact:true}).click();
      }
      assert.equal(visited.length,125);assert.equal(new Set(visited).size,125);
      await expect(navigation.getByRole("button",{name:"下一页",exact:true})).toBeDisabled();
      await navigation.getByRole("button",{name:"上一页",exact:true}).click();
      await expect(rows.first().getByRole("link")).toHaveAttribute("href",`#details/${users[50].id}`);
      const overviewBeforeDetails = overviewQueries.length;
      await page.evaluate(id=>{window.location.hash=`details/${id}`},users[124].id);
      await expect(page.getByRole("form",{name:"编辑备份实例 同名实例",exact:true})).toBeVisible();
      assert.equal(overviewQueries.length,overviewBeforeDetails,"details must query its ID without loading a list page");
      assert.equal(detailQueries.at(-1),users[124].id);
      await page.getByRole("button",{name:"实例列表",exact:true}).click();
      await expect(rows).toHaveCount(50);
      await navigation.getByRole("button",{name:"下一页",exact:true}).click();
      const removed = users[50].id;
      await expect(rows.first().getByRole("link")).toHaveAttribute("href",`#details/${removed}`);
      await rows.first().getByRole("button",{name:"删除",exact:true}).click();
      await rows.first().getByRole("button",{name:"确认删除",exact:true}).click();
      await expect(navigation.getByRole("button",{name:"上一页",exact:true})).toBeDisabled();
      await expect(statistics.getByRole("row").nth(1)).toContainText("125 / 62");
      await navigation.getByRole("button",{name:"下一页",exact:true}).click();
      await expect(rows.first().getByRole("link")).toHaveAttribute("href",`#details/${removed}`);
      await rows.first().getByRole("button",{name:"删除",exact:true}).click();
      await rows.first().getByRole("button",{name:"确认删除",exact:true}).click();
      await expect(statistics.getByRole("row").nth(1)).toContainText("124 / 62");
      await expect(navigation.getByRole("button",{name:"上一页",exact:true})).toBeDisabled();
      await page.getByRole("group",{name:"全局操作"}).getByRole("button",{name:"新建实例",exact:true}).click();
      await expect(statistics.getByRole("row").nth(1)).toContainText("125 / 62");
      await expect(navigation.getByRole("button",{name:"上一页",exact:true})).toBeDisabled();
      assert.equal(users.length,125);
      assert.ok(mutations.some(item => item.path.endsWith("/authorization")));
      assert.deepEqual(errors, []);
      console.log(`${engine.name()}: 125 same-name instances and 225 same-time logs, bounded bidirectional pages, global statistics, independent details, lifecycle changes, response budget, language and WCAG passed`);
      await context.close();
    } finally { await browser.close(); }
  }
} finally { await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve())); }
