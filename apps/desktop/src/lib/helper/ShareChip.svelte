<script lang="ts">
  import { helper } from "./state";
  import { helperUnshare } from "../ipc/helper";
  import { t } from "../i18n";
  let { pageId, width, height, onError }: { pageId: string; width: number; height: number; onError: () => void } = $props();
  let hover = $state(false), focus = $state(false);
  const share = $derived($helper?.share?.pageId === pageId ? $helper.share : null);
  async function stop() { try { await helperUnshare(); helper.update(s => s ? { ...s, share: null } : s); } catch { onError(); } }
</script>
{#if share}
  <div role="group" aria-label={$t("helper.share")} class="chip" onpointerenter={() => hover = true} onpointerleave={() => hover = false} onfocusin={() => focus = true} onfocusout={event => focus = event.currentTarget.contains(event.relatedTarget as Node)}>
    <span>{$t("helper.shared", { count: share.objectCount, page: share.pageNumber })}<small>{$t("helper.names", { names: $helper?.sessions.filter(s => share.sessionIds.includes(s.id)).map(s => s.clientName).join(", ") ?? "" })}</small></span>
    <button type="button" aria-label={$t("helper.unshare")} title={$t("helper.unshare")} onclick={stop}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg></button>
  </div>
  {#if hover || focus}<svg class="crop" viewBox={`0 0 ${width} ${height}`} aria-hidden="true"><rect x={share.cropPt.x} y={share.cropPt.y} width={share.cropPt.width} height={share.cropPt.height} /></svg>{/if}
{/if}
<style>
  .chip { position: absolute; z-index: 26; top: 8px; left: 8px; display: flex; align-items: center; gap: 4px; max-width: calc(100% - 16px); padding: 0 4px 0 12px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--panel); color: var(--text); box-shadow: var(--shadow-menu); font-size: var(--text-md); line-height: 1.4; }
  small { display: block; color: var(--muted); font-size: var(--text-sm); }
  button { display: grid; place-items: center; min-width: 44px; min-height: 44px; padding: 0; border: 0; border-radius: var(--radius); background: transparent; color: var(--muted); cursor: pointer; }
  button:hover { background: var(--wash); color: var(--text); }
  button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  button svg { width: var(--icon-dense); height: var(--icon-dense); fill: none; stroke: currentColor; stroke-width: var(--stroke-dense); stroke-linecap: round; }
  .crop { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; z-index: 25; overflow: visible; }
  .crop rect { fill: none; stroke: var(--paper-select); stroke-width: 2; stroke-dasharray: 6 4; vector-effect: non-scaling-stroke; }
</style>
