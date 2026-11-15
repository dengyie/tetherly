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
    // Scripted stream: the full-state burst a fresh session receives. Frames
    // use the real EventEnvelope wire shape: {"seq","type","data"} (§4.3).
    const frames = [
      { type: "snapshot", data: snapshot },
      { type: "activity.changed", data: activity },
    ];
    for (const f of frames) {
      res.write(`id: ${seq}\nevent: ${f.type}\ndata: ${JSON.stringify({ seq, type: f.type, data: f.data })}\n\n`);
      seq += 1;
    }
    const keep = setInterval(() => res.write(": ping\n\n"), 5000);
    req.on("close", () => clearInterval(keep));
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

  // SPA: /next/ and everything under it falls back to the entry document.
  let rel = p.startsWith("/next/") ? p.slice("/next/".length) : "";
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
