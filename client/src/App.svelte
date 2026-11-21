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
  import { brandLogo, brandName } from "./lib/brand";
  import Glass from "./lib/ui/Glass.svelte";
  import Icon from "./lib/ui/Icon.svelte";
  import StatusIndicator from "./lib/ui/StatusIndicator.svelte";
  import ThemeToggle from "./lib/ui/ThemeToggle.svelte";
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

  // Tray panel mode: the shell opens the 360×480 frameless window at index
  // with the #tray fragment (§6.3) — the compact single-screen view replaces
  // sidebar + header entirely.
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

  type Page = "overview" | "devices" | "transfers" | "inbox" | "activity" | "settings" | "layout" | "diagnostics";
  let page: Page = $state("overview");
  let paletteOpen = $state(false);

  // Order is the product's: everyday pages first, maintenance last.
  const nav: { id: Page; label: string; icon: string; hint: string }[] = [
    { id: "overview", label: "概览", icon: "home", hint: "本机状态、输入归属与配对" },
    { id: "devices", label: "设备", icon: "monitor", hint: "已配对与已屏蔽设备" },
    { id: "layout", label: "布局", icon: "layout", hint: "屏幕邻接与跨屏参数" },
    { id: "transfers", label: "传输", icon: "send", hint: "文件传输队列" },
    { id: "inbox", label: "收件箱", icon: "inbox", hint: "已接收的文件" },
    { id: "activity", label: "动态", icon: "activity", hint: "通知、消息与投递" },
    { id: "settings", label: "设置", icon: "settings", hint: "节点名称与共享策略" },
    { id: "diagnostics", label: "诊断", icon: "terminal", hint: "事件流与日志" },
  ];

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

  // Wording contract shared with the e2e suite: empty while live.
  const stateLabel: Record<string, string> = {
    connecting: "连接中…",
    live: "",
    resyncing: "重新同步…",
    offline: "连接中断，正在重连",
  };
  const linkStatus = $derived(
    $linkState === "live" ? "online" : $linkState === "offline" ? "offline" : "connecting",
  );
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
  <div class="shell h-screen app-backdrop" data-tray="panel">
    <Tray {devices} {input} {transfers} {settings} link={$linkState} {api} />
  </div>
{:else}
  <div class="shell flex h-screen overflow-hidden app-backdrop text-zinc-800 dark:text-zinc-100" data-tray="main">
    <Glass variant="sidebar" intensity="regular" interactive={false} class="w-56 shrink-0 flex flex-col">
      <div class="flex h-full flex-col">
        <div class="flex items-center gap-2.5 px-4 pt-4 pb-3">
          <img src={brandLogo} alt="" width="28" height="28" class="h-7 w-7 rounded-lg shadow-sm" draggable="false" />
          <div class="min-w-0">
            <div class="text-sm font-semibold leading-tight tracking-tight">{brandName}</div>
            <div class="text-[10px] uppercase tracking-wider text-zinc-400">键鼠共享网格</div>
          </div>
        </div>
        <nav class="flex flex-col gap-0.5 px-2 py-1" aria-label="主导航">
          {#each nav as n (n.id)}
            <button
              type="button"
              class="group/nav flex items-center gap-2.5 rounded-xl px-3 py-2 text-left text-[13px] font-medium transition-colors cursor-pointer {page === n.id
                ? 'bg-white dark:bg-zinc-800 text-zinc-900 dark:text-zinc-50 shadow-xs'
                : 'text-zinc-600 dark:text-zinc-400 hover:bg-white/60 dark:hover:bg-zinc-800/60 hover:text-zinc-900 dark:hover:text-zinc-100'}"
              aria-current={page === n.id ? "page" : undefined}
              onclick={() => (page = n.id)}
            >
              <Icon name={n.icon} size={16} class={page === n.id ? "text-brand-500" : "text-zinc-400 group-hover/nav:text-zinc-600 dark:group-hover/nav:text-zinc-300"} />
              {n.label}
            </button>
          {/each}
        </nav>
        <div class="mt-auto px-3 pb-3 pt-2 flex flex-col gap-2">
          <div class="flex items-center justify-between rounded-xl bg-white/50 dark:bg-zinc-800/50 px-3 py-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            <span class="inline-flex items-center gap-1.5"><Icon name="monitor" size={13} /><span class="mono">{onlineCount}</span> 在线</span>
            <span class="inline-flex items-center gap-1.5"><Icon name="send" size={13} /><span class="mono">{activeTransfers}</span> 传输</span>
          </div>
          <ThemeToggle compact class="w-full justify-between" />
        </div>
      </div>
    </Glass>

    <div class="flex min-w-0 flex-1 flex-col">
      <header class="flex h-12 shrink-0 items-center gap-3 border-b border-zinc-200/70 dark:border-zinc-800/80 bg-white/40 dark:bg-zinc-900/40 backdrop-blur-md px-5">
        <StatusIndicator status={linkStatus} label={linkText[$linkState]} />
        <span class="link text-xs text-zinc-500 dark:text-zinc-400" data-state={$linkState}>{stateLabel[$linkState]}</span>
        <div class="ml-auto flex items-center gap-2">
          <button type="button" class="btn btn-ghost btn-sm" aria-label="命令面板" title="命令面板 (⌘K)" onclick={() => (paletteOpen = true)}>
            <Icon name="search" size={14} />
            <kbd class="kbd">⌘K</kbd>
          </button>
        </div>
      </header>
      <main class="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        <div class="mx-auto w-full max-w-4xl flex flex-col gap-4">
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
      </main>
    </div>
    <CommandPalette bind:open={paletteOpen} items={paletteItems} onselect={onPalette} />
  </div>
{/if}
<ToastStack />
