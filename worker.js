// Only content-addressed WASM artifacts are exposed from the private R2 bucket.
export function selectEncoding(header = "") {
  const weights = new Map(header.toLowerCase().split(",").filter(Boolean).map(part => {
    const [name, ...params] = part.trim().split(";");
    const q = params.find(value => value.trim().startsWith("q="));
    const weight = q ? Number(q.trim().slice(2)) : 1;
    return [name.trim(), Number.isFinite(weight) ? Math.max(0, Math.min(1, weight)) : 0];
  }));
  const quality = name => weights.get(name) ?? (name === "identity" ? (weights.get("*") === 0 ? 0 : 1) : weights.get("*") ?? 0);
  return ["br", "gzip", "identity"].filter(name => quality(name) > 0)
    .sort((a, b) => quality(b) - quality(a))[0];
}

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (!/^\/wasm\/[a-f0-9]{64}\.wasm$/.test(url.pathname)) {
      return new Response("Not found", { status: 404 });
    }
    if (!["GET", "HEAD"].includes(request.method)) {
      return new Response("Method not allowed", { status: 405, headers: { Allow: "GET, HEAD" } });
    }
    // Cloudflare normalizes Accept-Encoding before invoking a Worker.
    const encoding = selectEncoding(request.cf?.clientAcceptEncoding ?? request.headers.get("Accept-Encoding") ?? "");
    if (!encoding) return new Response("No acceptable encoding", { status: 406 });
    const key = url.pathname.slice(1) + ({ br: ".br", gzip: ".gz", identity: "" }[encoding]);
    // Cache API does not use Vary to partition entries; explicitly isolate encodings.
    url.search = `?encoding=${encoding}&cache=2`;
    const cacheKey = new Request(url, { method: "GET" });
    try {
      let response = await caches.default.match(cacheKey);
      const hit = Boolean(response);
      if (!response) {
        const object = request.method === "HEAD" ? await env.WASM_BUCKET.head(key) : await env.WASM_BUCKET.get(key);
        if (!object) return new Response("WASM release not found", { status: 404 });
        const headers = new Headers({
          // Store compressed bytes as opaque data. Cache API can otherwise
          // transform encoded responses and cause a second compression layer.
          "Content-Type": "application/octet-stream",
          "Cache-Control": "public, max-age=31536000, immutable, no-transform",
          "ETag": object.httpEtag,
          "Vary": "Accept-Encoding",
          "X-Content-Type-Options": "nosniff",
        });
        response = new Response(request.method === "HEAD" ? null : object.body, { headers, encodeBody: "manual" });
        if (request.method === "GET") {
          ctx.waitUntil(caches.default.put(cacheKey, response.clone()).catch(error => console.error("WASM cache write failed", error)));
        }
      }
      const headers = new Headers(response.headers);
      headers.set("Content-Type", "application/wasm");
      if (encoding !== "identity") headers.set("Content-Encoding", encoding);
      headers.set("X-Wasm-Source", "r2");
      headers.set("X-Wasm-Cache", hit ? "HIT" : "MISS");
      const etag = headers.get("ETag");
      const matches = request.headers.get("If-None-Match")?.split(",").some(tag => tag.trim() === "*" || tag.trim().replace(/^W\//, "") === etag);
      if (matches || request.method === "HEAD") {
        if (response.body) ctx.waitUntil(response.body.cancel());
        return new Response(null, { status: matches ? 304 : 200, headers });
      }
      return new Response(response.body, { headers, encodeBody: "manual" });
    } catch (error) {
      console.error("WASM delivery failed", error);
      return new Response("WASM temporarily unavailable. Please retry.", { status: 503, headers: { "Cache-Control": "no-store", "Retry-After": "5" } });
    }
  },
};
