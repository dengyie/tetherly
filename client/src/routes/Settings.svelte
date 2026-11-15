<script lang="ts">
  import type { Settings } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";

  let { data, api }: { data: Settings | null; api: Api } = $props();

  // Local draft mirrors the event-fed store; PATCH result arrives back as a
  // settings.changed event, so edits only need to be held here between
  // submit and the round-trip.
  let nameDraft = $state("");
  let lastName = $state("");
  let saving = $state(false);
  let notice = $state("");
  $effect(() => {
    const n = data?.general.name ?? "";
    if (n !== lastName) {
      lastName = n;
      nameDraft = n;
    }
  });

  async function saveName() {
    const name = nameDraft.trim();
    if (!name || name === data?.general.name) return;
    saving = true;
    notice = "";
    try {
      await api.patchSettings({ name });
      notice = "名称已更新";
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  async function toggleAutoAccept() {
    if (!data) return;
    saving = true;
    notice = "";
    try {
      await api.patchSettings({ auto_accept: !data.transfer.auto_accept });
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<h1>设置</h1>
{#if notice}<p class="notice">{notice}</p>{/if}
<section class="card">
  <h2>常规</h2>
  {#if !data}
    <p class="dim">等待设置数据…</p>
  {:else}
    <form onsubmit={(e) => { e.preventDefault(); saveName(); }}>
      <label>
        <span class="k">节点名称</span>
        <input
          bind:value={nameDraft}
          maxlength={64}
          disabled={saving}
          placeholder={data.general.name}
        />
        <button type="submit" disabled={saving || !nameDraft.trim() || nameDraft.trim() === data.general.name}>
          保存
        </button>
      </label>
    </form>
    <ul>
      <li><span class="k">mDNS 广播</span><span>{data.general.advertise ? "开" : "关"}</span></li>
      <li><span class="k">自动重连</span><span>{data.general.reconnect ? "开" : "关"}</span></li>
      <li><span class="k">仅本机回环</span><span>{data.general.loopback_only ? "开" : "关"}</span></li>
      <li><span class="k">Overlay 组网</span><span>{data.general.overlay_enabled ? "开" : "关"}</span></li>
    </ul>
  {/if}
</section>
<section class="card">
  <h2>传输</h2>
  {#if !data}
    <p class="dim">等待设置数据…</p>
  {:else}
    <ul>
      <li>
        <span class="k">自动接收文件</span>
        <button class="toggle" disabled={saving} onclick={toggleAutoAccept}>
          {data.transfer.auto_accept ? "开（点击关闭）" : "关（点击开启）"}
        </button>
      </li>
    </ul>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); font-size: 13px; }
  .notice { color: var(--text-dim); font-size: 13px; }
  ul { list-style: none; margin: 8px 0 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 16px; align-items: baseline; }
  .k { color: var(--text-dim); width: 120px; flex: none; }
  form label { display: flex; gap: 10px; align-items: baseline; }
  input {
    flex: 1; max-width: 320px; font: inherit; font-size: 14px;
    background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: 6px; padding: 5px 10px;
  }
  button {
    border: 1px solid var(--border); background: var(--bg); color: var(--text);
    padding: 3px 10px; border-radius: 6px; cursor: pointer; font: inherit; font-size: 13px;
  }
  button:disabled { opacity: 0.5; cursor: default; }
</style>
