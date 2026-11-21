// Static server for Playwright e2e: serves client/dist at /next/ and answers
// the /api/v1 endpoints the SPA asks for with deterministic fixture JSON (no
// SSE loopback needed — the mock daemon pushes events via /api/v1/events as
// an infinite SSE stream that replays a scripted list then idles).
import http from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { join, extname } from "node:path";

const DIST = new URL("../dist", import.meta.url).pathname;
const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
};

const layout = {
  display: { width: 1920, height: 1080, scale_factor: 1 },
  edges: [
    { edge: "left", peer: null },
    { edge: "right", peer: "d1" },
    { edge: "top", peer: null },
    { edge: "bottom", peer: null },
  ],
  config: {
    resistance_px: 40,
    delay_ms: 150,
    corner_lock_size_px: 20,
    wrap_around: false,
    redundancy_band_px: 6,
    slow_dwell_ms: 200,
    fast_push_delta_px: 30,
    entry_return_grace_ms: 300,
    remote_speed_percent: 100,
  },
  policy: { enabled: true, allow_inbound_control: true, auto_link_paired_peers: false },
};

const settings = {
  general: { name: "e2e-mac", advertise: true, reconnect: true, loopback_only: false, overlay_enabled: false, tray_owner: "client" },
  transfer: { auto_accept: false },
};
const transfers = {
  active_count: 1,
  terminal_count: 1,
  transfers: [
    { transfer_id: "t1", peer: "d1", direction: "inbound", filename: "report.pdf", total: 1000, transferred: 620, speed_bps: 120000, phase: "transferring", error: null, started_ms: 0, updated_ms: 0 },
    { transfer_id: "t2", peer: "d1", direction: "outbound", filename: "photo.jpg", total: 2048, transferred: 2048, speed_bps: 0, phase: "completed", error: null, started_ms: 0, updated_ms: 0 },
  ],
  stats: { bytes_sent: 2048, bytes_received: 620, completed_count: 1, failed_count: 0 },
};
const inbox = { count: 1, total_bytes: 2048, files: [{ name: "photo.jpg", size: 2048, modified_at_secs: 1700000000, is_safe: true }] };

const snapshot = {
  system: {
    device_id: "self",
    name: "e2e-mac",
    platform: "macos",
    version: "0.2.58",
    api: 1,
    pid: 1,
    uptime_secs: 90,
    ports: { control: 45717, file: 45718, input: 45719, screen: 45720, ui: 4173 },
    peers: { live: 1, trusted: 1 },
    caps: [],
    features: [],
  },
  devices: [
    {
      id: "d1",
      name: "E2E-WIN",
      alias: null,
      platform: "windows",
      device_type: "desktop",
      online: true,
      trusted: true,
      blocked: false,
      path: "lan",
      addr: "192.168.1.9:45717",
      rtt_ms: 3,
      last_seen_ms: 1,
      caps: [],
      features: [],
      tags: [],
    },
  ],
  pairing: { pin_active: false, pin_expires_in_secs: 0 },
  input: { seat: "local", channel_open: false, peer: null },
};

const activity = {
  candidates: [
    {
      id: "c1",
      source: "d1",
      app_id: "com.bank",
      app_name: "Bank",
      uid: "42",
      title: "Your login code",
      has_otp: true,
      created_ms: Date.now() - 5000,
      expires_ms: Date.now() + 60000,
      actions: ["open"],
    },
  ],
  replies: [],
  messages: [
    {
      id: "m1",
      peer: "d1",
      direction: "inbound",
      text: "hello from e2e",
      timestamp_ms: Date.now() - 1000,
      read: false,
    },
  ],
  handoffs: [
    {
      id: "h1",
      sender_id: "d1",
      sender_alias: "E2E-WIN",
      url: "https://example.test/page",
      title: "a page",
      status: "pending",
      received_ms: Date.now() - 2000,
      opened_ms: null,
    },
  ],
};

let seq = 1;

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  const p = url.pathname;

  if (p === "/api/v1/events") {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "keep-alive",
    });
    // Scripted stream, mirroring the real daemon (§220): the subscribe burst
    // is the Snapshot ONLY — layout/settings/transfers/inbox/activity are
    // served by the GET routes below and pushed as `*.changed` only on
    // change. Frames use the real EventEnvelope wire shape
    // {"seq","type","data"} (§4.3), where `data` is the serde-tagged event
    // minus `type`, so the payload sits under its wrapper key
    // (`{snapshot: …}`, `{activity: …}` …), exactly as the daemon emits it.
    const frames = [{ type: "snapshot", data: { snapshot } }];
    for (const f of frames) {
      res.write(`id: ${seq}\nevent: ${f.type}\ndata: ${JSON.stringify({ seq, type: f.type, data: f.data })}\n\n`);
      seq += 1;
    }
    const keep = setInterval(() => res.write(": ping\n\n"), 5000);
    req.on("close", () => clearInterval(keep));
    return;
  }

  const sections = { layout, settings, transfers, inbox, activity };
  const sec = p.match(/^\/api\/v1\/(layout|settings|transfers|inbox|activity)$/);
  if (sec && req.method === "GET") {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify(sections[sec[1]]));
    return;
  }

  if (p === "/api/v1/health") {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ ok: true, api: 1, version: "0.2.58" }));
    return;
  }
  if (p.startsWith("/api/")) {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({}));
    return;
  }

  // SPA: served at / like the daemon (M4), with /next/ kept as an alias for
  // the historical e2e entry. Anything that is not a built file falls back
  // to the entry document.
  let rel = p.startsWith("/next/") ? p.slice("/next/".length) : p.replace(/^\//, "");
  if (p === "/next" || p === "/next/" || rel === "") rel = "index.html";
  const file = join(DIST, rel);
  if (existsSync(file)) {
    res.writeHead(200, { "content-type": MIME[extname(file)] || "application/octet-stream" });
    res.end(readFileSync(file));
  } else {
    res.writeHead(200, { "content-type": MIME[".html"] });
    res.end(readFileSync(join(DIST, "index.html")));
  }
});

server.listen(4173, "127.0.0.1", () => console.log("e2e server on 4173"));
