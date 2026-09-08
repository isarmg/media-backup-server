import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
const root = new URL("./", import.meta.url);
const provenance = JSON.parse(await readFile(new URL("provenance.json", root), "utf8"));
for (const [name, hash] of Object.entries(provenance.assets)) {
  assert.equal(createHash("sha256").update(await readFile(new URL(name, root))).digest("hex"), hash, name);
}
const sources = await readdir(new URL("../src/", root)).catch(error => { if (error.code === "ENOENT") return []; throw error; });
for (const name of sources) {
  if (!/\.[jt]sx?$/.test(name)) continue;
  const source = await readFile(new URL(`../src/${name}`, root), "utf8");
  assert.ok(!source.includes('from "@sarmg/admin-shell"'), "Use one shared Shell/context implementation throughout this product");
}
console.log("Reviewed Foundation Shell snapshot verified");
const policy = JSON.parse(await readFile(new URL("../../../foundation/platform-router.json", root), "utf8").catch(error => { if (error.code === "ENOENT") return "null"; throw error; }));
if (policy) {
  const router = await readFile(new URL("../../../foundation/platform_router.rs", root));
  assert.equal(createHash("sha256").update(router).digest("hex"), policy.sha256, "Foundation platform router snapshot");
  assert.equal(policy.diagnostics, false);
  assert.ok(!router.toString().includes("get(diagnostics"));
  console.log("Diagnostics-free Foundation platform router verified");
}
