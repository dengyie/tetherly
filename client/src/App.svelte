<script lang="ts">
  import { onMount } from "svelte";
  import { HttpTransport, setApiToken } from "./lib/transport/http";
  import { linkState, lastSeq } from "./lib/stores/connection";
  import { applyEvent, applySnapshot, initialDevices, type DevicesState } from "./lib/stores/devices";
  import type { Snapshot } from "./lib/api/gen/api";
  import Overview from "./routes/Overview.svelte";
  import Devices from "./routes/Devices.svelte";

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
  let page: "overview" | "devices" = $state("overview");

  onMount(() => {
    setApiToken(readFragmentToken() ?? sessionStorage.getItem("tetherly_token"));
    const transport = new HttpTransport();
    let state: DevicesState = initialDevices();
    let seq: number | null = null;
    const unsubscribe = transport.subscribe(
      null,
      (ev) => {
        seq = ev.id;
        lastSeq.set(ev.id);
        if (ev.type === "snapshot") state = applySnapshot(state, ev.data as Snapshot);
        state = applyEvent(state, ev);
        devices = state;
      },
      (s) => linkState.set(s),
    );
    void seq;
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
  </nav>
  <main>
    {#if page === "overview"}
      <Overview {devices} />
    {:else}
      <Devices {devices} />
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
