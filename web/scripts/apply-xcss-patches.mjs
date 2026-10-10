// Generated consumers use this same loader; no adjacent checkout is required.
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const digest = bytes => createHash("sha256").update(bytes).digest("hex");
const allowedFiles = new Set([
  "@xcss/web/dist/admin-shell/account.js",
  "@xcss/web/dist/admin-ui/content-blocks.css",
]);

function regularPath(root, relativePath, directory = false) {
  const parts = relativePath.split("/");
  if (parts.some(part => !part || part === "." || part === ".." || part.includes("\\"))) {
    throw new Error(`Invalid xcss patch path: ${relativePath}`);
  }
  let target = root;
  for (let index = 0; index < parts.length; index += 1) {
    target = resolve(target, parts[index]);
    const metadata = lstatSync(target);
    const needsDirectory = index < parts.length - 1 || directory;
    if (metadata.isSymbolicLink() || (needsDirectory ? !metadata.isDirectory() : !metadata.isFile())) {
      throw new Error(`xcss patch input must be a real ${needsDirectory ? "directory" : "file"}: ${relativePath}`);
    }
  }
  return target;
}

function editedBytes(bytes, edits) {
  let output = bytes;
  let previousOffset = bytes.length + 1;
  for (const edit of [...edits].reverse()) {
    const before = Buffer.from(edit.before);
    if (!Number.isSafeInteger(edit.offset) || edit.offset < 0 || edit.offset + before.length > bytes.length ||
        edit.offset + before.length > previousOffset || !bytes.subarray(edit.offset, edit.offset + before.length).equals(before)) {
      throw new Error("Invalid xcss patch edit or overlapping byte ranges");
    }
    output = Buffer.concat([output.subarray(0, edit.offset), Buffer.from(edit.after), output.subarray(edit.offset + before.length)]);
    previousOffset = edit.offset;
  }
  return output;
}

export function applyXcssPatches(root) {
  const metadataPath = regularPath(root, "patches/xcss.json");
  const manifest = JSON.parse(readFileSync(metadataPath, "utf8"));
  if (manifest.format !== 1 || manifest.xcssVersion !== "1.0.0" || manifest.files.length !== allowedFiles.size) {
    throw new Error("Unsupported xcss patch manifest; upgrade the reviewed patches with the dependency");
  }
  const lock = JSON.parse(readFileSync(regularPath(root, "package-lock.json"), "utf8"));
  const changes = [];
  const seen = new Set();
  for (const patch of manifest.files) {
    const identity = `${patch.package}/${patch.path}`;
    if (!allowedFiles.has(identity) || seen.has(identity)) {
      throw new Error(`Unsupported or duplicate xcss patch target: ${patch.package}/${patch.path}`);
    }
    seen.add(identity);
    const packageRoot = `node_modules/${patch.package}`;
    const installed = JSON.parse(readFileSync(regularPath(root, `${packageRoot}/package.json`), "utf8"));
    const locked = lock.packages?.[packageRoot];
    if (installed.name !== patch.package || installed.version !== manifest.xcssVersion ||
        locked?.version !== manifest.xcssVersion || locked.resolved !== patch.releaseUrl || locked.integrity !== patch.releaseIntegrity) {
      throw new Error(`xcss patch dependency identity changed: ${patch.package}; regenerate or remove this patch after the release upgrade`);
    }
    const target = regularPath(root, `${packageRoot}/${patch.path}`);
    const current = readFileSync(target);
    const currentDigest = digest(current);
    if (currentDigest === patch.patchedSha256) continue;
    if (currentDigest !== patch.baselineSha256) {
      throw new Error(`Unknown xcss patch baseline: ${patch.package}/${patch.path}; refusing to overwrite local or upgraded code`);
    }
    const updated = editedBytes(current, patch.edits);
    if (digest(updated) !== patch.patchedSha256) throw new Error(`xcss patch output hash mismatch: ${patch.package}/${patch.path}`);
    changes.push({ target, updated });
  }
  // Validate every baseline before modifying any installed package file.
  for (const { target, updated } of changes) writeFileSync(target, updated);
  console.log(`Reviewed xcss Web patches verified (${changes.length} applied)`);
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  if (root === sep) throw new Error("Refusing to patch a filesystem root");
  applyXcssPatches(root);
}
