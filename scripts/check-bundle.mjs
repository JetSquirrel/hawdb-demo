import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { run } from "../pkg/hawdb_wasm_demo.js";

assert.equal(typeof run, "function", "Generated bindings must export the GPUI entrypoint");
const bytes = await readFile(new URL("../pkg/hawdb_wasm_demo_bg.wasm", import.meta.url));
const module = new WebAssembly.Module(bytes);
assert(WebAssembly.Module.exports(module).some(({ name }) => name === "run"));
assert(!WebAssembly.Module.exports(module).some(({ name }) => name.includes("querybridge")));
console.log(`Unified WASM bundle verified (${(bytes.length / 1024 / 1024).toFixed(1)} MiB).`);
