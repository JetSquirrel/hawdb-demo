const bundle = { module: "./pkg/hawdb_wasm_demo.js", wasm: "./pkg/hawdb_wasm_demo_bg.wasm", bytes: 0 }; // BUILD_CONFIG
const loading = document.getElementById("loading");
const status = loading?.querySelector("span");
const progress = loading?.querySelector("progress");
const setStatus = message => { if (status) status.textContent = message; };
window.clearTimeout(window.bootWatchdog);
window.hawdbBootStarted = true;

async function withTimeout(promise, ms, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(message)), ms); })]);
  } finally { clearTimeout(timer); }
}

async function downloadWasm() {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 180_000);
  let idleTimer;
  const resetIdle = () => {
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => controller.abort(), 30_000);
  };
  try {
    resetIdle();
    const response = await fetch(bundle.wasm, { signal: controller.signal });
    if (!response.ok) throw new Error(`WASM download failed (HTTP ${response.status}).`);
    if (!response.headers.get("Content-Type")?.includes("application/wasm")) throw new Error("The server returned an invalid WASM response.");
    const reader = response.body.getReader();
    const chunks = [];
    let received = 0;
    // Fetch exposes decompressed bytes; Content-Length may describe compressed bytes.
    const total = bundle.bytes || (!response.headers.has("Content-Encoding") && Number(response.headers.get("Content-Length")));
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      resetIdle();
      chunks.push(value);
      received += value.length;
      setStatus(total ? `Downloading WASM… ${Math.min(100, Math.round(received / total * 100))}%` : `Downloading WASM… ${(received / 1048576).toFixed(1)} MiB`);
      if (progress && total) progress.value = received / total;
    }
    if (bundle.bytes && received !== bundle.bytes) throw new Error("WASM download is incomplete. Please retry.");
    const bytes = new Uint8Array(received);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
    return bytes;
  } catch (error) {
    if (controller.signal.aborted) throw new Error("WASM download timed out. Check your connection and retry.");
    throw error;
  } finally { clearTimeout(timer); clearTimeout(idleTimer); }
}

try {
  setStatus("Loading application code…");
  const { default: initialize, run } = await withTimeout(import(bundle.module), 30_000, "Application code download timed out. Please retry.");
  if (typeof run !== "function") throw new Error("The WASM bundle is out of date. Please reload.");
  setStatus("Connecting to WASM download…");
  const bytes = await downloadWasm();
  setStatus("Compiling WebAssembly…");
  if (progress) progress.removeAttribute("value");
  // Allow the compilation status to paint before entering the WASM runtime.
  await new Promise(resolve => setTimeout(resolve, 0));
  await withTimeout(initialize({ module_or_path: bytes }), 60_000, "WebAssembly initialization timed out. Please retry.");
  setStatus("Starting graph…");
  run();
  loading?.remove();
} catch (error) {
  console.error("Failed to start HawDB Graph Demo", error);
  window.showBootError(error instanceof Error ? error.message : String(error));
}
