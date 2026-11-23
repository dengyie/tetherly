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
  import { hydrateSections, sectionOf, type HydratedSection } from "./lib/stores/hydrate";
  import type { Snapshot } from "./lib/api/gen/api";
  import { Api } from "./lib/api/client";
  import type { Transport } from "./lib/transport/transport";
  import { brandLogo, brandName } from "./lib/brand";
  import Icon from "./lib/ui/Icon.svelte";
  import ToastStack from "./lib/ui/ToastStack.svelte";
  import CommandPalette, { type PaletteItem } from "./lib/ui/CommandPalette.svelte";
  import { applyTheme, type ThemeMode } from "./lib/ui/theme";
  import Overview from "./routes/Overview.svelte";
  import Devices from "./routes/Devices.svelte";
  import Diagnostics from "./routes/Diagnostics.svelte";
  import Transfers from "./routes/Transfers.svelte";
  import Inbox from "./routes/Inbox.svelte";
  import Layout from "./routes/Layout.svelte";
  import Settings from "./routes/Settings.svelte";
  import Activity from "./routes/Activity.svelte";
  import Tray from "./routes/Tray.svelte";

  // Tray panel mode: the shell (or the daemon's own menubar panel, §225)
  // opens the frameless 380-wide window at index with the #tray fragment
  // (§6.3) — the compact single-screen view replaces sidebar + header.
  // `#tray&token=…` is the daemon handoff form (token stripped below).
  const isTrayPanel = $state(/^#tray(?:&|$)/.test(location.hash));
  // Lets app.css make html/body transparent for the frosted native panel.
  document.documentElement.dataset.tray = isTrayPanel ? "panel" : "main";

  // Token arrives via the daemon URL fragment (#token=… or #tray&token=…);
  // read and strip it immediately so it never stays in the address bar or
  // history. The #tray marker is kept so a reload stays in panel mode.
  function readFragmentToken(): string | null {
    const m = /(?:^#|&)token=([^&]*)/.exec(location.hash);
    if (!m) return null;
    const t = decodeURIComponent(m[1]);
    const keep = isTrayPanel ? "#tray" : "";
    history.replaceState(null, "", location.pathname + location.search + keep);
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

  type Page = "overview" | "devices" | "transfers" | "inbox" | "activity" | "settings" | "layout" | "diagnostics";
  let page: Page = $state("overview");
  let paletteOpen = $state(false);

  // Magpie-style grouped sidebar (§224): everyday pages first, maintenance
  // last, 设置 pinned to the bottom like Magpie's 設定.
  type NavItem = { id: Page; label: string; icon: string; hint: string; group: string };
  const groups: { label: string; items: NavItem[] }[] = [
    {
      label: "使用",
      items: [
        { id: "overview", label: "概览", icon: "home", hint: "本机状态、输入归属与配对", group: "使用" },
        { id: "devices", label: "设备", icon: "monitor", hint: "已配对与已屏蔽设备", group: "使用" },
        { id: "layout", label: "布局", icon: "layout", hint: "屏幕邻接与跨屏参数", group: "使用" },
      ],
    },
    {
      label: "传输",
      items: [
        { id: "transfers", label: "传输", icon: "send", hint: "文件传输队列", group: "传输" },
        { id: "inbox", label: "收件箱", icon: "inbox", hint: "已接收的文件", group: "传输" },
        { id: "activity", label: "动态", icon: "activity", hint: "通知、消息与投递", group: "传输" },
      ],
    },
    {
      label: "维护",
      items: [{ id: "diagnostics", label: "诊断", icon: "terminal", hint: "事件流与日志", group: "维护" }],
    },
  ];
  const settingsItem: NavItem = { id: "settings", label: "设置", icon: "settings", hint: "外观、节点名称与共享策略", group: "系统" };
  const nav: NavItem[] = [...groups.flatMap((g) => g.items), settingsItem];
  const current = $derived(nav.find((n) => n.id === page) ?? nav[0]);

  // macOS Tauri shell draws the traffic lights over the sidebar
  // (titleBarStyle=Overlay); leave room for them. Windows ignores that flag.
  const macOverlay = isTauri() && /Mac/i.test(navigator.platform);
  const daemonAddr = isTauri() ? "127.0.0.1:45716" : location.host;

  const paletteItems: PaletteItem[] = $derived([
    ...nav.map((n) => ({ id: `go:${n.id}`, label: n.label, hint: n.hint, group: "页面", icon: n.icon })),
    { id: "theme:auto", label: "主题：跟随系统", group: "外观", icon: "monitor" },
    { id: "theme:light", label: "主题：浅色", group: "外观", icon: "sun" },
    { id: "theme:dark", label: "主题：深色", group: "外观", icon: "moon" },
  ]);
  function onPalette(item: PaletteItem) {
    if (item.id.startsWith("go:")) page = item.id.slice(3) as Page;
    else if (item.id.startsWith("theme:")) applyTheme(item.id.slice(6) as ThemeMode);
  }
  function onShellKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      paletteOpen = !paletteOpen;
    }
  }

  onMount(() => {
    setApiToken(readFragmentToken() ?? sessionStorage.getItem("tetherly_token"));
    // In the Tauri shell the Rust side owns the token and the SSE
    // connection; the browser build keeps same-origin fetch + fragment auth.
    const t: Transport = transport ?? (isTauri() ? new TauriTransport() : new HttpTransport());
    const client = new Api(t);
    api = client;
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
    // Per-section count of real `*.changed` frames: hydration compares it
    // before/after its fetch and drops a result the stream has outrun.
    const changed: Record<HydratedSection, number> = { layout: 0, settings: 0, transfers: 0, inbox: 0, activity: 0 };
    const apply = (ev: { id: number; type: string; data: unknown }) => {
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
    };
    let stopped = false;
    // §220: the daemon's subscribe burst is the Snapshot alone. In the Tauri
    // shell the SSE handshake can complete before the WebView's listeners
    // exist, so that first snapshot frame can be lost even though the
    // connection is healthy — every event-fed page would then sit on its
    // waiting placeholder forever (production 2026-11-23). Remember whether
    // a snapshot arrived; the first live/resyncing state that lands WITHOUT
    // one triggers the same REST hydration the snapshot path uses.
    let sawSnapshot = false;
    const unsubscribe = t.subscribe(
      null,
      (ev) => {
        lastSeq.set(ev.id);
        const sec = sectionOf(ev.type);
        if (sec) changed[sec] += 1;
        apply(ev);
        if (ev.type === "snapshot") {
          sawSnapshot = true;
          // The daemon's burst is the Snapshot only (§220): pull the five
          // event-fed sections over REST once per (re)connection.
          const before = { ...changed };
          void hydrateSections(
            client,
            (section) => stopped || changed[section] !== before[section],
            apply,
            ev.id,
          );
        }
      },
      (st) => {
        linkState.set(st);
        if ((st === "live" || st === "resyncing") && !sawSnapshot) {
          sawSnapshot = true; // hydrate once; a real snapshot supersedes us.
          const before = { ...changed };
          void hydrateSections(
            client,
            (section) => stopped || changed[section] !== before[section],
            apply,
            0,
          );
        }
      },
    );
    return () => {
      stopped = true;
      unsubscribe();
    };
  });

  // Wording contract shared with the e2e suite: empty while live.
  const stateLabel: Record<string, string> = {
    connecting: "连接中…",
    live: "",
    resyncing: "重新同步…",
    offline: "连接中断，正在重连",
  };
  const linkText: Record<string, string> = {
    connecting: "连接中",
    live: "已连接",
    resyncing: "同步中",
    offline: "已断开",
  };
  const onlineCount = $derived([...devices.byId.values()].filter((d) => d.online && d.trusted).length);
  const activeTransfers = $derived(
    transfers.transfers?.transfers.filter((t) => t.phase === "transferring" || t.phase === "verifying").length ?? 0,
  );
</script>

<svelte:window onkeydown={isTrayPanel ? undefined : onShellKey} />

{#if isTrayPanel}
  <div class="shell h-screen" data-tray="panel">
    <Tray {devices} {input} {transfers} {settings} link={$linkState} {api} />
  </div>
{:else}
  <div class="shell flex h-screen overflow-hidden" data-tray="main">
    <aside class="side {macOverlay ? 'pt-9' : ''}">
      <div class="side-head" data-tauri-drag-region>
        <img src={brandLogo} alt="" width="22" height="22" draggable="false" />
        <span>{brandName}</span>
      </div>
      <nav class="flex flex-col" aria-label="主导航">
        {#each groups as g (g.label)}
          <div class="nav-gl">{g.label}</div>
          {#each g.items as n (n.id)}
            <button type="button" class="nav-btn {page === n.id ? 'on' : ''}" title={n.hint} aria-current={page === n.id ? "page" : undefined} onclick={() => (page = n.id)}>
              <span class="nav-ic"><Icon name={n.icon} size={15} /></span>
              {n.label}
              <!-- counters are decorative (aria-hidden) so the button's accessible name stays the bare label -->
              {#if n.id === "devices" && onlineCount > 0}<span class="cnt" aria-hidden="true">{onlineCount}</span>{/if}
              {#if n.id === "transfers" && activeTransfers > 0}<span class="cnt" aria-hidden="true">{activeTransfers}</span>{/if}
            </button>
          {/each}
        {/each}
      </nav>
      <div class="side-foot">
        <button type="button" class="nav-btn {page === 'settings' ? 'on' : ''}" aria-current={page === "settings" ? "page" : undefined} onclick={() => (page = "settings")}>
          <span class="nav-ic"><Icon name="settings" size={15} /></span>
          设置
        </button>
        <div class="gw" title="守护进程 {daemonAddr}">
          <span class="dot" data-state={$linkState}></span>
          <span>{linkText[$linkState]}</span>
          <span class="mono truncate">{daemonAddr}</span>
        </div>
      </div>
    </aside>

    <main class="flex min-w-0 flex-1 flex-col">
      <header class="topbar" data-tauri-drag-region>
        <div class="crumb">
          <span>{current.group}</span><i>/</i><h1>{current.label}</h1>
        </div>
        <span class="link" data-state={$linkState}>{stateLabel[$linkState]}</span>
        <div class="ml-auto flex items-center gap-2">
          <button type="button" class="search-pill" aria-label="命令面板" title="命令面板 (⌘K)" onclick={() => (paletteOpen = true)}>
            <Icon name="search" size={13} />
            <span>搜索或跳转</span>
            <kbd class="kbd">⌘K</kbd>
          </button>
        </div>
      </header>
      <div class="min-h-0 flex-1 overflow-y-auto px-5 pb-6 pt-1">
        <!-- Magpie: content hugs the breadcrumb's left edge and spans the pane, capped for ultra-wide. -->
        <div class="w-full max-w-[1180px] flex flex-col gap-3">
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
          {:else if page === "layout" && api}
            <Layout data={layout.layout} {devices} {api} />
          {:else if page === "diagnostics" && api}
            <Diagnostics {api} />
          {/if}
        </div>
      </div>
    </main>
    <CommandPalette bind:open={paletteOpen} items={paletteItems} onselect={onPalette} />
  </div>
{/if}
<ToastStack />
