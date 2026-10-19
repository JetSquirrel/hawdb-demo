import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { brotliCompressSync, brotliDecompressSync, gzipSync, gunzipSync, constants } from "node:zlib";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = resolve(root, "dist");
const artifacts = resolve(root, "target/r2");
const hash = data => createHash("sha256").update(data).digest("hex");
const wasm = await readFile(resolve(root, "pkg/hawdb_wasm_demo_bg.wasm"));
const wasmHash = hash(wasm);
const key = `wasm/${wasmHash}.wasm`;
await mkdir(artifacts, { recursive: true });
async function compressed(suffix, compress, decompress) {
  try {
    const cached = await readFile(resolve(artifacts, `${wasmHash}.wasm${suffix}`));
    if (decompress(cached).equals(wasm)) return cached;
  } catch { /* Missing or damaged cache: regenerate it. */ }
  return compress(wasm);
}
// The content hash keeps each release immutable and makes rollbacks safe.
const variants = [
  ["", wasm],
  [".gz", await compressed(".gz", data => gzipSync(data, { level: 9 }), gunzipSync)],
  [".br", await compressed(".br", data => brotliCompressSync(data, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }), brotliDecompressSync)],
];
const objects = [];
for (const [suffix, bytes] of variants) {
  const file = resolve(artifacts, `${wasmHash}.wasm${suffix}`);
  await writeFile(file, bytes);
  objects.push({ key: key + suffix, file, bytes: bytes.length, sha256: hash(bytes) });
}
await writeFile(resolve(artifacts, "manifest.json"), JSON.stringify({ objects }, null, 2));
await rm(output, { recursive: true, force: true });
// Publish only explicitly selected browser files, never source or build caches.
for (const file of ["index.html", "styles.css", "assets/OFL.txt"]) {
  const destination = resolve(output, file);
  await mkdir(dirname(destination), { recursive: true });
  await copyFile(resolve(root, file), destination);
}
const glue = await readFile(resolve(root, "pkg/hawdb_wasm_demo.js"));
const gluePath = `pkg/${hash(glue)}.js`;
await mkdir(resolve(output, "pkg"));
await writeFile(resolve(output, gluePath), glue);
const config = { module: `./${gluePath}`, wasm: `/${key}?delivery=2`, bytes: wasm.length };
const app = (await readFile(resolve(root, "app.js"), "utf8"))
  .replace(/const bundle = .*; \/\/ BUILD_CONFIG/, `const bundle = ${JSON.stringify(config)}; // BUILD_CONFIG`);
const appPath = `app.${hash(app)}.js`;
await writeFile(resolve(output, appPath), app);
const html = (await readFile(resolve(output, "index.html"), "utf8")).replace('src="./app.js"', `src="./${appPath}"`);
await writeFile(resolve(output, "index.html"), html);
await writeFile(resolve(output, "_headers"), "/pkg/*\n  Cache-Control: public, max-age=31536000, immutable\n");
console.log(`Site packaged; R2 WASM: ${wasm.length} bytes, Brotli: ${objects[2].bytes} bytes`);
