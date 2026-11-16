<script lang="ts">
  import { onMount } from "svelte";
  import { HttpTransport, setApiToken } from "./lib/transport/http";
  import { TauriTransport, isTauri } from "./lib/transport/tauri";
  import { linkState, lastSeq } from "./lib/stores/connection";
  import { applyEvent, applySnapshot, initialDevices, type DevicesState } from "./lib/stores/devices";
  import { applyEvent as sysEvent, applySnapshot as sysSnapshot, initialSystem, type SystemState } from "./lib/stores/system";
  import { applyEvent as pairEvent, applySnapshot as pairSnapshot, initialPairing, type PairingState } from "./lib/stores/pairing";
  import { applyEvent as inputEvent, applySnapshot as inputSnapshot, initialInput, type InputState } from "./lib/stores/input";
  import { applyEvent as transfersEvent, initialTransfers, type TransfersState } from "./lib/stores/transfers";
  import { applyEvent as inboxEvent, initialInbox, type InboxState } from "./lib/stores/inbox";
  import { applyEvent as layoutEvent, initialLayout, type LayoutState } from "./lib/stores/layout";
  import { applyEvent as settingsEvent, initialSettings, type SettingsState } from "./lib/stores/settings";
  import { applyEvent as activityEvent, initialActivity, type ActivityState } from "./lib/stores/activity";
  import type { Snapshot } from "./lib/api/gen/api";
  import { Api } from "./lib/api/client";
  import type { Transport } from "./lib/transport/transport";
  import Overview from "./routes/Overview.svelte";
  import Devices from "./routes/Devices.svelte";
  import Diagnostics from "./routes/Diagnostics.svelte";
  import Transfers from "./routes/Transfers.svelte";
  import Inbox from "./routes/Inbox.svelte";
  import Layout from "./routes/Layout.svelte";
  import Settings from "./routes/Settings.svelte";
  import Activity from "./routes/Activity.svelte";
  import Tray from "./routes/Tray.svelte";

  // Tray panel mode: the shell opens the 360×480 frameless window at index
  // with the #tray fragment (§6.3) — the compact single-screen view replaces
  // header + navigation entirely.
  const isTrayPanel = $state(location.hash === "#tray");

  // Token arrives via the daemon URL fragment (#token=…); read and strip it
  // immediately so it never stays in the address bar or history.
  function readFragmentToken(): string | null {
    if (!location.hash.startsWith("#token=")) return null;
    const t = decodeURIComponent(location.hash.slice(7));
    history.replaceState(null, "", location.pathname + location.search);
    sessionStorage.setItem("tetherly_token", t);
    return t;
  }

  let { transport = null }: { transport?: Transport | null } = $props();

  let devices: DevicesState = $state(initialDevices());
  let system: SystemState = $state(initialSystem());
  let pairing: PairingState = $state(initialPairing());
  let input: InputState = $state(initialInput());
  let transfers: TransfersState = $state(initialTransfers());
  let inbox: InboxState = $state(initialInbox());
  let layout: LayoutState = $state(initialLayout());
  let settings: SettingsState = $state(initialSettings());
  let activity: ActivityState = $state(initialActivity());
  let api = $state<Api | null>(null);
  let page: "overview" | "devices" | "transfers" | "inbox" | "activity" | "settings" | "layout" | "diagnostics" = $state("overview");

  onMount(() => {
    setApiToken(readFragmentToken() ?? sessionStorage.getItem("tetherly_token"));
    // In the Tauri shell the Rust side owns the token and the SSE
    // connection; the browser build keeps same-origin fetch + fragment auth.
    const t: Transport = transport ?? (isTauri() ? new TauriTransport() : new HttpTransport());
    api = new Api(t);
    // Reducer mirrors: mutated inside the event callback, assigned to the
    // $state variables so Svelte 5 picks up the change.
    let d = initialDevices();
    let s = initialSystem();
    let p = initialPairing();
    let i = initialInput();
    let tr = initialTransfers();
    let ib = initialInbox();
    let la = initialLayout();
    let se = initialSettings();
    let ac = initialActivity();
    const unsubscribe = t.subscribe(
      null,
      (ev) => {
        lastSeq.set(ev.id);
        if (ev.type === "snapshot") {
          const snap = ev.data as Snapshot;
          d = applySnapshot(d, snap);
          s = sysSnapshot(s, snap);
          p = pairSnapshot(p, snap);
          i = inputSnapshot(i, snap);
        } else {
          d = applyEvent(d, ev);
          s = sysEvent(s, ev);
          p = pairEvent(p, ev);
          i = inputEvent(i, ev);
          tr = transfersEvent(tr, ev);
          ib = inboxEvent(ib, ev);
          la = layoutEvent(la, ev);
          se = settingsEvent(se, ev);
          ac = activityEvent(ac, ev);
        }
        devices = d;
        system = s;
        pairing = p;
        input = i;
        transfers = tr;
        inbox = ib;
        layout = la;
        settings = se;
        activity = ac;
      },
      (st) => linkState.set(st),
    );
    return unsubscribe;
  });

  const stateLabel: Record<string, string> = {
    connecting: "连接中…",
    live: "",
    resyncing: "重新同步…",
    offline: "连接中断，正在重连",
  };
</script>

<div class="shell" data-tray={isTrayPanel ? "panel" : "main"}>
  {#if isTrayPanel}
    <Tray {devices} {input} {transfers} {settings} link={$linkState} {api} />
  {:else}
  <header>
    <span class="brand">Tetherly</span>
    <span class="link" data-state={$linkState}>{stateLabel[$linkState]}</span>
  </header>
  <nav>
    <button class:active={page === "overview"} onclick={() => (page = "overview")}>概览</button>
    <button class:active={page === "devices"} onclick={() => (page = "devices")}>设备</button>
    <button class:active={page === "transfers"} onclick={() => (page = "transfers")}>传输</button>
    <button class:active={page === "inbox"} onclick={() => (page = "inbox")}>收件箱</button>
    <button class:active={page === "activity"} onclick={() => (page = "activity")}>动态</button>
    <button class:active={page === "settings"} onclick={() => (page = "settings")}>设置</button>
    <button class:active={page === "layout"} onclick={() => (page = "layout")}>布局</button>
    <button class:active={page === "diagnostics"} onclick={() => (page = "diagnostics")}>诊断</button>
  </nav>
  <main>
    {#if page === "overview" && api}
      <Overview {devices} {system} {input} {pairing} {api} />
    {:else if page === "devices" && api}
      <Devices list={[...devices.byId.values()]} {api} />
    {:else if page === "transfers" && api}
      <Transfers data={transfers.transfers} {api} />
    {:else if page === "inbox" && api}
      <Inbox data={inbox.inbox} {api} />
    {:else if page === "activity"}
      <Activity data={activity.activity} />
    {:else if page === "settings" && api}
      <Settings data={settings.settings} {api} />
    {:else if page === "layout"}
      <Layout data={layout.layout} />
    {:else if page === "diagnostics" && api}
      <Diagnostics {api} />
    {/if}
  </main>
  {/if}
</div>

<style>
  .shell { min-height: 100vh; display: flex; flex-direction: column; }
  header {
    display: flex; align-items: center; gap: 12px;
    padding: 10px 16px; background: var(--surface); border-bottom: 1px solid var(--border);
  }
  .brand { font-weight: 600; font-size: 15px; }
  .link { margin-left: auto; color: var(--text-dim); font-size: 12px; }
  nav { display: flex; gap: 4px; padding: 8px 16px; }
  nav button {
    border: none; background: transparent; color: var(--text-dim);
    padding: 6px 12px; border-radius: 8px; cursor: pointer; font: inherit;
  }
  nav button.active { background: var(--surface); color: var(--text); }
  main { padding: 16px; max-width: 880px; width: 100%; margin: 0 auto; flex: 1; }
</style>
