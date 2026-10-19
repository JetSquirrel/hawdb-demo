import test from 'node:test';
import assert from 'node:assert/strict';
import worker, { selectEncoding } from '../worker.js';
const path = `/wasm/${'a'.repeat(64)}.wasm`;
const url = `https://hawdb-demo.jetsquirrel.cloud${path}`;

test('encoding negotiation honors exclusions and preferences', () => {
  assert.equal(selectEncoding('gzip, br'), 'br');
  assert.equal(selectEncoding('br;q=0, gzip'), 'gzip');
  assert.equal(selectEncoding(''), 'identity');
  assert.equal(selectEncoding('identity;q=0, *;q=0'), undefined);
  assert.equal(selectEncoding('gzip;q=0.8, br;q=0.5, identity;q=0'), 'gzip');
});

test('R2 responses are cached separately per encoding; HEAD and conditional requests have no body', async () => {
  const entries = new Map();
  const reads = [];
  const pending = [];
  globalThis.caches = { default: {
    match: async request => { const entry = entries.get(request.url); return entry && new Response(entry.body, { headers: entry.headers }); },
    put: async (request, response) => { entries.set(request.url, { body: await response.arrayBuffer(), headers: response.headers }); },
  } };
  const env = { WASM_BUCKET: {
    get: async key => { reads.push(key); return { body: new TextEncoder().encode(key), httpEtag: `"${key}"` }; },
    head: async key => ({ httpEtag: `"${key}"` }),
  } };
  const ctx = { waitUntil: promise => pending.push(promise) };
  const send = (headers = {}, method = 'GET') => worker.fetch(new Request(url, { headers, method }), env, ctx);
  const br = await send({ 'Accept-Encoding': 'br' });
  assert.equal(br.headers.get('Content-Encoding'), 'br');
  assert.equal(br.headers.get('X-Wasm-Source'), 'r2');
  assert.equal(br.headers.get('X-Wasm-Cache'), 'MISS');
  assert.match(br.headers.get('Cache-Control'), /immutable/);
  assert.equal(await br.text(), path.slice(1) + '.br');
  await Promise.all(pending);
  const cached = [...entries.values()][0];
  assert.equal(cached.headers.get('Content-Encoding'), null);
  assert.equal(cached.headers.get('Content-Type'), 'application/octet-stream');
  const hit = await send({ 'Accept-Encoding': 'br' });
  assert.equal(hit.headers.get('X-Wasm-Cache'), 'HIT');
  await hit.text();
  const normalized = new Request(url, { headers: { 'Accept-Encoding': 'br, gzip' } });
  Object.defineProperty(normalized, 'cf', { value: { clientAcceptEncoding: 'identity' } });
  const raw = await worker.fetch(normalized, env, ctx);
  assert.equal(raw.headers.get('Content-Encoding'), null);
  assert.equal(await raw.text(), path.slice(1));
  const gz = await send({ 'Accept-Encoding': 'gzip' });
  assert.equal(gz.headers.get('Content-Encoding'), 'gzip');
  await gz.text();
  assert.equal(reads.length, 3);
  const head = await send({ 'Accept-Encoding': 'br' }, 'HEAD');
  assert.equal(await head.text(), '');
  const conditional = await send({ 'Accept-Encoding': 'br', 'If-None-Match': br.headers.get('ETag') });
  assert.equal(conditional.status, 304);
  assert.equal(await conditional.text(), '');
  await Promise.all(pending);
});

test('invalid paths, methods and missing releases fail without caching', async () => {
  const env = { WASM_BUCKET: { get: async () => null } };
  globalThis.caches = { default: { match: async () => undefined } };
  const ctx = { waitUntil() { assert.fail('must not cache errors'); } };
  assert.equal((await worker.fetch(new Request(url.replace('/wasm/', '/private/')), env, ctx)).status, 404);
  assert.equal((await worker.fetch(new Request(url, { method: 'POST' }), env, ctx)).status, 405);
  assert.equal((await worker.fetch(new Request(url), env, ctx)).status, 404);
  assert.equal((await worker.fetch(new Request(url, { headers: { 'Accept-Encoding': '*;q=0' } }), env, ctx)).status, 406);
});

// Run the shipped loader with controlled network failures; no browser globals or
// real network are needed to assert that failures cannot leave an endless spinner.
async function runLoader(fetchResponse) {
  const { readFile } = await import('node:fs/promises');
  const source = (await readFile(new URL('../app.js', import.meta.url), 'utf8'))
    .replace('import(bundle.module)', 'loadModule()');
  let error;
  let runs = 0;
  const status = {};
  const progress = { removeAttribute() {} };
  const loading = { querySelector: selector => selector === 'span' ? status : progress, remove() {} };
  const window = { clearTimeout, showBootError: message => { error = message; } };
  const document = { getElementById: () => loading };
  const AsyncFunction = Object.getPrototypeOf(async function() {}).constructor;
  await new AsyncFunction('document', 'window', 'fetch', 'loadModule', 'setTimeout', 'clearTimeout', 'console', source)(
    document, window, fetchResponse,
    async () => ({ default: async () => {}, run: () => { runs++; } }),
    (fn, ms) => setTimeout(fn, ms >= 30_000 ? 10 : ms), clearTimeout, { error() {} },
  );
  return { error, runs };
}

test('loader reports HTTP errors and never starts the runtime', async () => {
  const { error, runs } = await runLoader(async () => new Response('Unavailable', { status: 503 }));
  assert.match(error, /HTTP 503/);
  assert.equal(runs, 0);
});

test('loader aborts a stalled download and exposes a retryable error', async () => {
  let aborted = false;
  const { error, runs } = await runLoader((_, { signal }) => new Promise((_, reject) => {
    signal.addEventListener('abort', () => { aborted = true; reject(new Error('Aborted')); }, { once: true });
  }));
  assert.equal(aborted, true);
  assert.match(error, /timed out/);
  assert.equal(runs, 0);
});
