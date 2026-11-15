// Typed client for the /api/v1 contract (ADR-139 §4.2). One method per
// endpoint; pages never build fetch calls themselves. Everything rides on
// the Transport seam so the same methods work under MockTransport tests and
// the future Tauri shell.
import type {
  BlockRequest,
  ConnectRequest,
  Device,
  DevicePatch,
  Diagnostics,
  Health,
  Inbox,
  InboxOpenRequest,
  Input,
  LogTail,
  LogLevel,
  LogLevelRequest,
  Pairing,
  PinGrant,
  Settings,
  SettingsPatch,
  System,
  Transfers,
} from "./gen/api";
import type { Transport } from "../transport/transport";

export class Api {
  constructor(private readonly t: Transport) {}

  // Open to everyone: lets the client tell "daemon down" from "token wrong".
  health(): Promise<Health> {
    return this.t.request("GET", "/api/v1/health");
  }

  // No GET /snapshot: the initial state arrives as the first event-stream
  // frame; the store layer owns it.
  system(): Promise<System> {
    return this.t.request("GET", "/api/v1/system");
  }
  devices(): Promise<Device[]> {
    return this.t.request("GET", "/api/v1/devices");
  }
  device(id: string): Promise<Device> {
    return this.t.request("GET", `/api/v1/devices/${encodeURIComponent(id)}`);
  }
  patchDevice(id: string, patch: DevicePatch): Promise<Device> {
    return this.t.request("PATCH", `/api/v1/devices/${encodeURIComponent(id)}`, patch);
  }
  forgetDevice(id: string): Promise<void> {
    return this.t.request("DELETE", `/api/v1/devices/${encodeURIComponent(id)}`);
  }
  connectDevice(id: string, req: ConnectRequest = {}): Promise<void> {
    return this.t.request("POST", `/api/v1/devices/${encodeURIComponent(id)}/connect`, req);
  }
  blockDevice(id: string, req: BlockRequest = {}): Promise<void> {
    return this.t.request("POST", `/api/v1/devices/${encodeURIComponent(id)}/block`, req);
  }
  unblockDevice(id: string): Promise<void> {
    return this.t.request("DELETE", `/api/v1/devices/${encodeURIComponent(id)}/block`);
  }

  pairing(): Promise<Pairing> {
    return this.t.request("GET", "/api/v1/pairing");
  }
  pin(): Promise<PinGrant> {
    return this.t.request("GET", "/api/v1/pairing/pin");
  }
  rotatePin(): Promise<PinGrant> {
    return this.t.request("POST", "/api/v1/pairing/pin/rotate");
  }
  revokePin(): Promise<void> {
    return this.t.request("DELETE", "/api/v1/pairing/pin");
  }

  input(): Promise<Input> {
    return this.t.request("GET", "/api/v1/input");
  }
  releaseInput(): Promise<void> {
    return this.t.request("POST", "/api/v1/input/release");
  }

  transfers(): Promise<Transfers> {
    return this.t.request("GET", "/api/v1/transfers");
  }
  cancelTransfer(id: string): Promise<Transfers> {
    return this.t.request("POST", `/api/v1/transfers/${encodeURIComponent(id)}/cancel`);
  }
  clearTransfers(): Promise<Transfers> {
    return this.t.request("DELETE", "/api/v1/transfers");
  }

  inbox(): Promise<Inbox> {
    return this.t.request("GET", "/api/v1/inbox");
  }
  openInboxFile(name: string, req: InboxOpenRequest = { name }): Promise<void> {
    return this.t.request("POST", `/api/v1/inbox/${encodeURIComponent(name)}/open`, req);
  }
  deleteInboxFile(name: string): Promise<Inbox> {
    return this.t.request("DELETE", `/api/v1/inbox/${encodeURIComponent(name)}`);
  }
  clearInbox(): Promise<Inbox> {
    return this.t.request("DELETE", "/api/v1/inbox");
  }

  settings(): Promise<Settings> {
    return this.t.request("GET", "/api/v1/settings");
  }
  patchSettings(patch: SettingsPatch): Promise<Settings> {
    return this.t.request("PATCH", "/api/v1/settings", patch);
  }

  diagnostics(): Promise<Diagnostics> {
    return this.t.request("GET", "/api/v1/diagnostics");
  }
  logs(lines = 200): Promise<LogTail> {
    return this.t.request("GET", `/api/v1/diagnostics/logs?lines=${lines}`);
  }
  setLogLevel(req: LogLevelRequest): Promise<LogLevel> {
    return this.t.request("PATCH", "/api/v1/diagnostics/log-level", req);
  }
}
