import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
const { objects } = JSON.parse(await readFile(new URL("../target/r2/manifest.json", import.meta.url)));
for (const object of objects) {
  const bytes = await readFile(object.file);
  if (createHash("sha256").update(bytes).digest("hex") !== object.sha256) throw new Error(`Artifact changed: ${object.key}`);
  const result = spawnSync("wrangler", ["r2", "object", "put", `hawdb-demo-assets/${object.key}`, "--remote", "--file", object.file,
    "--content-type", "application/wasm", "--cache-control", "public, max-age=31536000, immutable"], { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
