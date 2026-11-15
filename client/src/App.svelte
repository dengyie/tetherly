<script lang="ts">
  import { onMount } from "svelte";
  import { HttpTransport, setApiToken } from "./lib/transport/http";
  import { linkState, lastSeq } from "./lib/stores/connection";
  import { applyEvent, applySnapshot, initialDevices, type DevicesState } from "./lib/stores/devices";
  import { applyEvent as sysEvent, applySnapshot as sysSnapshot, initialSystem, type SystemState } from "./lib/stores/system";
  import { applyEvent as pairEvent, applySnapshot as pairSnapshot, initialPairing, type PairingState } from "./lib/stores/pairing";
  import { applyEvent as inputEvent, applySnapshot as inputSnapshot, initialInput, type InputState } from "./lib/stores/input";
  import type { Snapshot } from "./lib/api/gen/api";
  import { Api } from "./lib/api/client";
  import Overview from "./routes/Overview.svelte";
  import Devices from "./routes/Devices.svelte";
  import Diagnostics from "./routes/Diagnostics.svelte";

  // Token arrives via the daemon URL fragment (#token=…); read and strip it
  // immediately so it never stays in the address bar or history.
  function readFragmentToken(): string | null {
    if (!location.hash.startsWith("#token=")) return null;
    const t = decodeURIComponent(location.hash.slice(7));
    history.replaceState(null, "", location.pathname + location.search);
    sessionStorage.setItem("tetherly_token", t);
    return t;
  }

  let devices: DevicesState = $state(initialDevices());
  let system: SystemState = $state(initialSystem());
  let pairing: PairingState = $state(initialPairing());
  let input: InputState = $state(initialInput());
  let api = $state<Api | null>(null);
  let page: "overview" | "devices" | "diagnostics" = $state("overview");

  onMount(() => {
    setApiToken(readFragmentToken() ?? sessionStorage.getItem("tetherly_token"));
    const transport = new HttpTransport();
    api = new Api(transport);
    // Reducer mirrors: mutated inside the event callback, assigned to the
    // $state variables so Svelte 5 picks up the change.
    let d = initialDevices();
    let s = initialSystem();
    let p = initialPairing();
    let i = initialInput();
    const unsubscribe = transport.subscribe(
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
        }
        devices = d;
        system = s;
        pairing = p;
        input = i;
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

<div class="shell">
  <header>
    <span class="brand">Tetherly</span>
    <span class="link" data-state={$linkState}>{stateLabel[$linkState]}</span>
  </header>
  <nav>
    <button class:active={page === "overview"} onclick={() => (page = "overview")}>概览</button>
    <button class:active={page === "devices"} onclick={() => (page = "devices")}>设备</button>
    <button class:active={page === "diagnostics"} onclick={() => (page = "diagnostics")}>诊断</button>
  </nav>
  <main>
    {#if page === "overview" && api}
      <Overview {devices} {system} {input} {pairing} {api} />
    {:else if page === "devices" && api}
      <Devices list={[...devices.byId.values()]} {api} />
    {:else if page === "diagnostics" && api}
      <Diagnostics {api} />
    {/if}
  </main>
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
