// Benchmark the **yggdryl** Node binding against Node's built-ins on identical
// workloads — the core `http` client (HTTP) and `zlib` (compression) — and print
// a markdown results table. Same high-level operation through both, same
// in-process server / in-memory payload ("same code, two backends").
//
//     (cd bindings/node && npm run build) && node benchmarks/compare.mjs

import http from "node:http";
import zlib from "node:zlib";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { HttpSession, Compression } = require("../bindings/node");

async function timedAsync(fn, iters) {
  for (let i = 0; i < Math.max(1, iters / 10); i++) await fn();
  const start = process.hrtime.bigint();
  for (let i = 0; i < iters; i++) await fn();
  return Number(process.hrtime.bigint() - start) / 1e9 / iters; // seconds/call
}

function timed(fn, iters) {
  for (let i = 0; i < Math.max(1, iters / 10); i++) fn();
  const start = process.hrtime.bigint();
  for (let i = 0; i < iters; i++) fn();
  return Number(process.hrtime.bigint() - start) / 1e9 / iters;
}

const mibps = (n, secs) => n / (1024 * 1024) / secs;

function table(title, header, rows) {
  console.log(`\n### ${title}\n`);
  console.log("| " + header.join(" | ") + " |");
  console.log("|" + header.map(() => "---").join("|") + "|");
  for (const row of rows) console.log("| " + row.join(" | ") + " |");
}

// Node's core http client GET into a single Buffer (the requests-equivalent).
function nodeGet(agent, url) {
  return new Promise((resolve, reject) => {
    http.get(url, { agent }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => resolve(Buffer.concat(chunks)));
      res.on("error", reject);
    });
  });
}

async function httpBench() {
  const big = Buffer.alloc(8 * 1024 * 1024).map((_, i) => i % 251);
  const small = Buffer.from("small-response-body");
  const server = http.createServer((req, res) => {
    const body = req.url === "/big" ? big : small;
    res.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "Content-Length": body.length,
    });
    res.end(body);
  });
  server.on("connection", (s) => s.setNoDelay(true)); // fair fight, no delayed-ACK
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const rows = [];
  try {
    const yg = new HttpSession();
    const agent = new http.Agent({ keepAlive: true });

    let ygT = await timedAsync(async () => (await yg.get(base + "/small")).content, 400);
    let ndT = await timedAsync(() => nodeGet(agent, base + "/small"), 400);
    rows.push([
      "GET small body (latency)",
      `${(ygT * 1e3).toFixed(3)} ms`,
      `${(ndT * 1e3).toFixed(3)} ms`,
      `${(ndT / ygT).toFixed(2)}×`,
    ]);

    const n = big.length;
    ygT = await timedAsync(async () => (await yg.get(base + "/big")).content, 20);
    ndT = await timedAsync(() => nodeGet(agent, base + "/big"), 20);
    rows.push([
      "GET 8 MiB body (throughput)",
      `${mibps(n, ygT).toFixed(0)} MiB/s`,
      `${mibps(n, ndT).toFixed(0)} MiB/s`,
      `${(ndT / ygT).toFixed(2)}×`,
    ]);
  } finally {
    server.close();
  }
  table("HTTP — yggdryl vs node:http (same in-process server)", ["workload", "yggdryl", "node http", "speedup"], rows);
}

function compressionBench() {
  let csv = "col_a,col_b,col_c\n";
  for (let i = 0; i < 150000; i++) csv += `${i},${i * 2},value_${i % 97}\n`;
  const payload = Buffer.from(csv);
  const n = payload.length;
  const rows = [];

  const gz = Compression.fromStr("gzip");
  let ygT = timed(() => gz.compress(payload), 10);
  let ndT = timed(() => zlib.gzipSync(payload), 10);
  rows.push(["gzip compress", `${mibps(n, ygT).toFixed(0)} MiB/s`, `${mibps(n, ndT).toFixed(0)} MiB/s`, `${(ndT / ygT).toFixed(2)}×`]);

  const packedYg = gz.compress(payload);
  const packedNd = zlib.gzipSync(payload);
  ygT = timed(() => gz.decompress(packedYg), 50);
  ndT = timed(() => zlib.gunzipSync(packedNd), 50);
  rows.push(["gzip decompress", `${mibps(n, ygT).toFixed(0)} MiB/s`, `${mibps(n, ndT).toFixed(0)} MiB/s`, `${(ndT / ygT).toFixed(2)}×`]);
  table(`Compression — yggdryl vs node:zlib (${(n / 1024) | 0} KiB CSV payload)`, ["workload", "yggdryl", "node zlib", "speedup"], rows);

  // Codecs node's zlib does not ship (zstd before Node 22's experimental zstd,
  // snappy never; brotli is built-in but Snappy/Zstd/Brotli all live here in one API).
  const extra = [];
  for (const name of ["zstd", "snappy", "brotli"]) {
    const codec = Compression.fromStr(name);
    if (!codec.isAvailable) continue;
    const packed = codec.compress(payload);
    const ct = timed(() => codec.compress(payload), 10);
    const dt = timed(() => codec.decompress(packed), 50);
    extra.push([name, `${mibps(n, ct).toFixed(0)} MiB/s`, `${mibps(n, dt).toFixed(0)} MiB/s`, `${(n / packed.length).toFixed(2)}×`]);
  }
  if (extra.length) table("Bonus — codecs with no built-in node:zlib equivalent", ["codec", "compress", "decompress", "ratio"], extra);
}

console.log("# yggdryl vs Node — same code, measured\n");
console.log(
  "_The thin Node binding runs the bulk work (an HTTP download, a whole-buffer " +
    "compress) in Rust in one call. For tiny per-call work the FFI + Promise " +
    "crossing dominates — use the bulk / streaming methods, and see the Rust-core " +
    "`cargo bench` numbers for the true ceiling._",
);
await httpBench();
compressionBench();
