# HawDB Graph Demo

A browser demo that compiles a real in-memory [HawDB](https://github.com/nowledge-co/hawdb) database and the real [`gpui-graph`](https://github.com/JetSquirrel/gpui-graph) frontend into one WebAssembly module.

Licensed under the [MIT License](LICENSE). Third-party dependencies retain their
own licenses; bundled fonts are covered by the [SIL Open Font License](assets/OFL.txt).

## Architecture

- **Database:** HawDB commit `032296335928261a17d7c98088cee7e1c869957c`, built with `default-features = false`.
- **Graph frontend:** `gpui-graph` commit `76bb32e34a8570319db0c91d051914f7b1b45feb`, using `NetworkGraph` directly—no JavaScript graph renderer or behavior port.
- **Browser backend:** `gpui-base 0.7.3`, `gpui-pre 0.3.9`, and `gpui-pre-web 0.3.9`. GPUI owns the document canvas and renders through WebGPU with WebGL2 fallback.
- **One module:** HawDB, Cypher execution, GPUI state, force layout, graph painting, input handling, toolbar, and inspector are compiled into `pkg/hawdb_wasm_demo_bg.wasm`. `app.js` only initializes the module and calls `run()`.
- **Execution model:** single-threaded browser main thread. This keeps one WASM instance and avoids SharedArrayBuffer/cross-origin-isolation requirements. Large synchronous queries can block UI frames, so this remains a development demo.

The database is ephemeral. Reloading creates a new in-memory database and reloads the bundled graph and `sightings` SQL table. The graph sample includes one isolated node to verify that zero-degree entities render.

## Cloudflare deployment

Production: https://hawdb-demo.jetsquirrel.cloud

HTML, CSS and JavaScript use Cloudflare Workers Static Assets. WASM is served
from the private `hawdb-demo-assets` R2 bucket through the same domain's `/wasm/`
route, with edge caching and precompressed Brotli/gzip variants. Database
execution and rendering remain entirely in the browser. The custom domain and
R2 binding are configured in `wrangler.jsonc`.

With Wrangler 4 installed and logged in to the configured Cloudflare account:

```bash
npm run deploy
```

`npm run deploy` builds the release, uploads all three WASM representations to
R2, then publishes the Worker and the explicit browser-file allowlist in `dist/`.
Use this command for releases; a bare `wrangler deploy` does not upload new R2
objects. Upload failures stop deployment. The R2 bucket must exist beforehand
(`wrangler r2 bucket create hawdb-demo-assets`); public bucket access is not needed.
Rust sources, build caches, and project configuration are not published.
`wrangler deploy --dry-run` checks the package without uploading or deploying.

WASM object keys and JavaScript filenames contain SHA-256 content hashes to
prevent stale JS/WASM pairs. WASM responses use one-year immutable caching;
cache keys isolate Brotli, gzip and identity encodings. Keep old R2 releases to
support open tabs and rollback. The Worker streams bytes without decompressing
or recompiling them. `npm run test:delivery` checks encoding negotiation, cache
separation, conditional/HEAD requests and error responses.

The loading screen shows download progress and compilation/startup stages, with
a retry button on failure. Downloads abort after 30 seconds without data or
180 seconds total; code loading and initialization also have timeouts. A small
inline watchdog reports when the JavaScript loader itself cannot load.

## Run locally

Requirements: nightly Rust, `wasm32-unknown-unknown`, wasm-bindgen CLI 0.2.128, Python 3, Node 22+, and either Homebrew LLVM or Zig for HawDB's C dependency. Apple Clang/ar cannot build the WASM Zstd archive.

```sh
rustup toolchain install nightly
rustup target add wasm32-unknown-unknown --toolchain nightly
rustup component add llvm-tools --toolchain nightly
cargo install wasm-bindgen-cli --version 0.2.128 --locked
npm ci
npm run build
npm run serve
```

Open <http://127.0.0.1:8080>. GPUI Web requires an HTTP origin; do not use `file://`.

## Terminal → graph

The app opens in **Terminal**, with a `hawdb>` prompt and a scrollable transcript.
Choose **CYPHER** or **SQL** explicitly with the language button, then enter a
statement. **Graph example** and **SQL example** load runnable queries without
executing them.

- **Enter / Run:** execute once and append the command, result, or error.
- **Shift+Enter:** insert a newline without executing.
- **↑ / ↓:** browse history from a single-line draft; **Alt+↑ / Alt+↓** also work
  for multiline commands. Returning past the newest entry restores the unsent
  draft and its language.
- **Draw graph →:** plot the returned node/relationship objects and open **Split**.
  The graph is a saved snapshot; drawing a historical result never reruns its query.
- **Split:** terminal and graph together; subsequent graph-shaped results update
  the graph. Scalar/table results leave the last plotted graph unchanged. Its
  source query is displayed above the canvas.
- **Graph:** full graph canvas and inspector. **Terminal:** full terminal again.
- **Clear:** clear only the terminal history. **Sample:** reset both graph and SQL
  data, history, selection, and query inputs.

Try these Cypher statements individually:

```cypher
MATCH (n) RETURN n
MATCH (a {id: 'hawdb'})-[r]->(b) RETURN a, r, b
CREATE (:Hawk {name: 'Aurelia'})
MATCH (h:Hawk) RETURN h
```

`RETURN n` draws returned nodes; return relationships as well (`a, r, b`) to draw
edges. Terminal plotting uses HawDB's internal `_id` and `labels`, so custom `id`
properties are not required. A relationship returned without its endpoint objects
gets placeholder nodes; return the endpoints to see their labels and properties.
Nested returned lists/maps are supported. Graphs are capped at 2,000 combined
nodes and relationships, with an explicit message if the limit is exceeded.

Switch to SQL, or click **SQL example**, then run:

```sql
SELECT * FROM sightings ORDER BY id
SELECT species, SUM(count) AS total FROM sightings GROUP BY species
```

The terminal keeps the last 40 commands, including errors. Each table preview is
limited to 100 rows, 12 columns, and 240 characters per cell. These display limits
do not limit query execution. Both languages operate on the same HawDB instance.

## Graph workspace

The quick-query toolbar in **Graph** accepts Cypher. Press Enter or **Run** to
execute it, then the graph refreshes from the entire database. This original
snapshot adapter requires a unique custom `id` property on each node; use terminal
result plotting for native node identities. The graph supports node dragging,
wheel zoom, panning, selection, arrow-key navigation, and `0` to fit.

Quick-query results appear below the graph. The inspector is hidden below 560 px
to leave room for the graph. Split uses columns at widths of 1,100 px and above,
and stacks the terminal above the graph in narrower windows.

Try these queries one at a time:

```cypher
MATCH (n) RETURN n.name AS name, n.kind AS kind ORDER BY name
CREATE (:Entity {id: 'demo', name: 'New idea', kind: 'Knowledge'})
MATCH (a {id: 'hawdb'}), (b {id: 'demo'}) CREATE (a)-[:RELATED {kind: 'STORES'}]->(b)
MATCH (n {id: 'demo'}) DETACH DELETE n
```

Queries are limited to 64 KiB. In the Graph quick-query toolbar, a successful write
that creates missing or duplicate visual IDs clears the graph and shows an error;
the database change remains committed. Repair the IDs or use **Sample** to start
over. Terminal commands retain errors inline and never erase previous results.

The bundled IBM Plex Sans and JetBrains Mono fonts are distributed under the SIL
Open Font License in `assets/OFL.txt`; no runtime font download is needed.

## Validate

```sh
npm run check
```

This checks Rust formatting, runs native HawDB-to-`GraphData` and result-preview tests, builds and links the unified release WASM module, runs wasm-bindgen and Binaryen, syntax-checks the JavaScript loader, and verifies the generated module's entrypoint. Browser rendering still requires a WebGPU- or WebGL2-capable browser.

Release builds use Rust `opt-level = "z"`, fat LTO and one codegen unit, followed
by the lockfile-pinned Binaryen `wasm-opt -Oz`. The optimizer validates the result
and checks that public exports remain intact and no new imports are introduced.
It only replaces the staged WASM when the result is smaller. This reduces the
actual executable artifact; Cloudflare's HTTP compression is applied separately.
Validated optimizer results are cached under `target/wasm-opt/`, keyed by the
input binary, optimizer script and Binaryen package version. Repeated builds
reuse the result after validating it again.

The current optimized WASM is 15.70 MiB (16,465,668 bytes), down from 22.83 MiB.
The original edge-compressed Brotli response measured 5.64 MiB. R2 build-time
Brotli quality 11 produces 4.22 MiB (4,424,256 bytes) for the same optimized WASM.

For browser validation, run the default query, create and delete a node, select a node to inspect it, enter invalid Cypher, and restore **Reset sample**. Confirm the graph, result preview, and status update and that the browser console has no runtime errors. The build keeps the previous bundle if wasm-bindgen or optimization fails; if the loader reports an outdated bundle, rerun `npm run build` and reload.
