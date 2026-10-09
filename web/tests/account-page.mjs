import assert from 'node:assert/strict';
import { expect } from '@playwright/test';

export async function checkAccountPage(page) {
  const trigger = page.getByRole('banner').getByRole('button', { name: '账号设置', exact: true });
  const appearance = node => ({ image: node.querySelector('svg').innerHTML, decoration: getComputedStyle(node).textDecorationLine,
    size: node.querySelector('svg').getBoundingClientRect().width });
  await page.mouse.move(0, 0);
  const before = await trigger.evaluate(appearance), fonts = [];
  const requested = request => { if (new URL(request.url()).pathname.endsWith('.woff2')) fonts.push(request.url()); };
  page.on('request', requested);
  await trigger.click(); await page.mouse.move(0, 0);
  const account = page.getByRole('region', { name: '账号设置', exact: true });
  await expect(account).toBeVisible();
  await expect(page).toHaveURL(/#account$/);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.xcss-header-navigation [aria-pressed="true"]')).toHaveCount(0);
  assert.deepEqual(await trigger.evaluate(appearance), before);
  assert.equal(before.decoration, 'none');
  await expect(account.locator('.xcss-form-field > span')).toHaveText(['用户名', '当前密码', '新密码', '确认新密码']);
  await expect(account.getByRole('button')).toHaveText(['保存']);
  await account.getByLabel('当前密码', { exact: true }).fill('discarded-account-draft');
  await page.getByRole('button', { name: '实例列表', exact: true }).click();
  await expect(account).toHaveCount(0);
  await trigger.click();
  await expect(account.getByLabel('当前密码', { exact: true })).toHaveValue('');
  await page.getByRole('button', { name: '实例列表', exact: true }).click();
  await expect(page.getByRole('table', { name: '实例统计', exact: true })).toBeVisible();
  assert.equal(fonts.length, 0, 'Account navigation must not reload any font');
  page.off('request', requested);
}
