import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";

const root = new URL("./", import.meta.url);
const provenance = JSON.parse(await readFile(new URL("provenance.json", root), "utf8"));
assert.equal(provenance.cjk.sha256, "80bf6db8920b2999d900e08f9e5031f7686baeb72dbdb80891f2c1ae9cec606f");
for (const [name, expected] of Object.entries(provenance.assets)) {
  const bytes = await readFile(new URL(name, root));
  assert.equal(createHash("sha256").update(bytes).digest("hex"), expected, name);
  if (name.endsWith(".woff2")) {
    assert.equal(bytes.subarray(0, 4).toString(), "wOF2", name);
    assert.ok(bytes.length <= 256 * 1024, name);
  }
}
const cjk = (await readdir(new URL("cjk/", root))).map(name => `cjk/${name}`).sort();
assert.deepEqual(cjk, Object.keys(provenance.assets).filter(name => name.startsWith("cjk/")).sort());
const css = await readFile(new URL("fonts.css", root), "utf8");
assert.ok(css.includes('font-variant-ligatures:none'));
assert.ok(css.includes('"calt" 0,"liga" 0,"clig" 0,"dlig" 0'));
assert.ok(css.includes('font-style:normal;font-weight:400'));
assert.ok(css.includes('font-style:normal;font-weight:700'));
assert.ok(!css.includes('font-weight:100 900'));
assert.ok(css.includes('MapleMonoNormalNL-Regular.woff2'));
assert.ok(css.includes('MapleMonoNormalNL-Bold.woff2'));
assert.ok(!css.includes('Italic.woff2'));
assert.equal(provenance.latin.handwriting, false);
assert.ok(!css.includes('url("./MapleMono.woff2")'));
console.log("Default fonts: Normal NL upright Regular/Bold, no ligatures, unchanged CJK coverage");
