import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const input = process.argv[2];
assert(input, "Usage: node scripts/optimize-wasm.mjs <input.wasm>");
const output = `${input}.optimized`;
const original = await readFile(input);
const before = new WebAssembly.Module(original);
const optimizer = fileURLToPath(new URL("../node_modules/binaryen/bin/wasm-opt", import.meta.url));
const signature = (item) => JSON.stringify(item);
const cacheDirectory = new URL("../target/wasm-opt/", import.meta.url);
const cacheKey = createHash("sha256").update(original)
  .update(await readFile(fileURLToPath(import.meta.url)))
  .update(await readFile(new URL("../node_modules/binaryen/package.json", import.meta.url)))
  .digest("hex");
const cachedFile = new URL(`${cacheKey}.wasm`, cacheDirectory);

function validate(optimized) {
  const after = new WebAssembly.Module(optimized);
  assert.deepEqual(
    WebAssembly.Module.exports(after).map(signature).sort(),
    WebAssembly.Module.exports(before).map(signature).sort(),
    "Optimization changed the public WASM exports",
  );
  const imports = new Set(WebAssembly.Module.imports(before).map(signature));
  assert(WebAssembly.Module.imports(after).every((item) => imports.has(signature(item))),
    "Optimization introduced an import missing from the generated bindings");
}

let cached;
try {
  cached = await readFile(cachedFile);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (cached) {
  validate(cached);
  assert(cached.length <= original.length, "Invalid optimizer cache size");
  await writeFile(input, cached);
  console.log(`wasm-opt cache: ${original.length} → ${cached.length} bytes`);
  process.exit(0);
}

try {
  await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [
      optimizer, input, "-Oz",
      // Rust/wasm-bindgen already use these features. Do not enable experimental
      // proposals or transformations that assume traps cannot occur.
      "--enable-bulk-memory", "--enable-reference-types", "--enable-multivalue",
      "--enable-nontrapping-float-to-int", "--enable-sign-ext",
      "--strip-debug", "--strip-producers", "-o", output,
    ], { stdio: "inherit" });
    child.once("error", reject);
    child.once("exit", (code) => code === 0 ? resolve() : reject(new Error(`wasm-opt exited with ${code}`)));
  });
  const optimized = await readFile(output);
  validate(optimized);
  await mkdir(cacheDirectory, { recursive: true });
  const cacheStaging = new URL(`${cacheKey}.${process.pid}.tmp`, cacheDirectory);
  await writeFile(cacheStaging, optimized.length < original.length ? optimized : original);
  await rename(cacheStaging, cachedFile);
  if (optimized.length < original.length) {
    await rename(output, input);
    console.log(`wasm-opt: ${original.length} → ${optimized.length} bytes (${((1 - optimized.length / original.length) * 100).toFixed(1)}% smaller)`);
  } else {
    console.log("wasm-opt did not reduce size; keeping the original validated binary");
  }
} finally {
  await rm(output, { force: true });
}
