import assert from "node:assert/strict";
import "../fonts/verify.mjs";
import { existsSync, readFileSync } from "node:fs";

import { assertSarmgWebToolchain } from "@sarmg/web-toolchain";
import manifest from "../package.json" with { type: "json" };

const lock = JSON.parse(
  readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"),
);
// One exact published Foundation version, with tarball integrity and no local links.
const foundationPackages = ["admin-web", "admin-shell", "admin-ui", "contracts", "design-tokens", "http-client", "web-fonts", "web-toolchain"];

const nodeVersion = readFileSync(
  new URL("../../.node-version", import.meta.url),
  "utf8",
);
assert.match(nodeVersion, /^26\.7\.0\n?$/);
assertSarmgWebToolchain(manifest, nodeVersion);
for (const name of foundationPackages) {
  const dependency = `@sarmg/${name}`;
  const expected = `https://github.com/isarmg/sarmg-foundation-server/releases/download/v0.9.1/sarmg-${name}-0.9.1.tgz`;
  assert.equal(manifest.dependencies?.[dependency], expected);
  assert.equal(lock.packages?.[""]?.dependencies?.[dependency], expected);

  const locked = lock.packages?.[`node_modules/${dependency}`];
  assert.equal(locked?.link, undefined);
  assert.equal(locked?.resolved, expected);
  assert.equal(locked?.version, "0.9.1");
  assert.match(locked?.integrity ?? "", /^sha512-[A-Za-z0-9+/]+={0,2}$/);
}
const adminStyles = readFileSync(new URL("../node_modules/@sarmg/admin-ui/dist/styles.css", import.meta.url), "utf8");
assert.match(adminStyles, /@import ["']\.\/content-blocks\.css["']/);
for (const snapshot of ["content-blocks.css", "provenance.json", "verify.mjs"]) {
  assert.equal(existsSync(new URL(`../appearance/${snapshot}`, import.meta.url)), false, "Foundation CSS must not be copied into the product");
}

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
assert.match(main, /createSarmgAdminApplication/);
assert.doesNotMatch(main, /AdministratorsPanel|平台管理员|Platform administrators/);
assert.doesNotMatch(main, /useAdministratorSession|function Login|localStorage|sessionStorage|error\.message/);
assert.doesNotMatch(styles, /@font-face|font-family|--sarmg-[a-z-]+\s*:|\.toast|\.sidebar/);
