# xcss Web 源码修复

这些补丁来自 xcss 公共源码，修复账号页的冗余标题、描述和表格布局。`xcss.json` 记录源码路径与 SHA-256、原发行包 URL/integrity、修改前后 SHA-256 和最小字节编辑。补丁只作用于 `@xcss/web/admin-shell/dist/account.js` 与 `@xcss/web/admin-ui/dist/content-blocks.css`。

已发布的 1.0.0 包与锁文件保持原样。构建入口在 TypeScript/Vite 前明确运行 `scripts/apply-xcss-patches.mjs`；`npm ci --ignore-scripts` 后重新构建仍会应用修复。相同输出可重复校验，未知版本、锁身份或基线直接报错；所有输入验证完成才写入，不覆盖本地修改。独立检出不依赖相邻 xcss 仓库。

唯一生成来源是 xcss 的 `scripts/export-consumer-web-patches.mjs` 与 `scripts/consumer-patches/apply-xcss-patches.mjs`。在公共源码编译后，用保留原发行输入的消费者作基线，再显式传入消费者 package 根目录：

```sh
node scripts/export-consumer-web-patches.mjs --baseline <原始消费者 package 根目录> <目标 package 根目录>...
```

后续升级到正式包含这些修复的 xcss 发行版时，先验证实际 Web 行为，再一起移除这份补丁、加载器和构建入口。不要修改旧发行 URL/integrity 或让过时补丁静默继续作用于新版本。
