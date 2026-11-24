<script lang="ts">
  import { t } from "../i18n";
  import { tick } from "svelte";
  import { cycledTab, type NotebookTab } from "./tabs";

  type Props = {
    tabs: NotebookTab[];
    activeRoot: string;
    switchingRoot: string | null;
    busy: boolean;
    saving: boolean;
    warning: boolean;
    onSelect: (root: string) => Promise<boolean>;
    onClose: (root: string) => Promise<void>;
  };

  let {
    tabs,
    activeRoot,
    switchingRoot,
    busy,
    saving,
    warning,
    onSelect,
    onClose,
  }: Props = $props();
  let tablist: HTMLElement;

  async function select(root: string) {
    if (!(await onSelect(root))) return;
    await tick();
    tablist
      ?.querySelector<HTMLButtonElement>(`[data-tab-root="${CSS.escape(root)}"]`)
      ?.focus();
  }

  function keydown(event: KeyboardEvent) {
    const offset = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : null;
    if (!offset) return;
    const next = cycledTab(tabs, activeRoot, offset);
    if (!next) return;
    event.preventDefault();
    void select(next);
  }
</script>

<div
  bind:this={tablist}
  class="notebook-tabs"
  data-ui="solid"
  role="tablist"
  aria-label={$t("workspace.openNotebooks")}
  tabindex="-1"
  onkeydown={keydown}
>
  {#each tabs as tab (tab.root)}
    {@const active = tab.root === activeRoot}
    <div class:active class:switching={switchingRoot === tab.root} class="notebook-tab">
      <button
        class="tab-target"
        type="button"
        role="tab"
        aria-selected={active}
        tabindex={active ? 0 : -1}
        data-tab-root={tab.root}
        title={tab.title}
        disabled={busy}
        onclick={() => void select(tab.root)}
      >
        {#if active}
          <span class:warning class:saving class="state-dot"></span>
        {/if}
        <span class="tab-title">{tab.title}</span>
      </button>
      <button
        class="tab-close"
        type="button"
        aria-label={$t("workspace.close", { "title": tab.title })}
        title={$t("workspace.close", { "title": tab.title })}
        disabled={busy}
        onclick={() => void onClose(tab.root)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
      </button>
    </div>
  {/each}
</div>

<style>
  .notebook-tabs {
    display: flex;
    min-width: 0;
    /* Only as wide as the tabs themselves, so "open another notebook" sits next to them. */
    flex: 0 1 auto;
    overflow-x: auto;
    gap: 4px;
    scrollbar-width: none;
  }
  .notebook-tabs::-webkit-scrollbar { display: none; }
  .notebook-tab {
    display: flex;
    min-width: 132px;
    max-width: 220px;
    height: var(--control);
    flex: 1 1 180px;
    align-items: center;
    border-radius: var(--radius);
    background: transparent;
    transition: background var(--fast) ease;
  }
  .notebook-tab:hover { background: var(--wash); }
  /* Selected: fill plus inset ring, nothing else. */
  .notebook-tab.active { background: var(--accent-fill); box-shadow: inset 0 0 0 1px var(--accent-edge); }
  .notebook-tab.switching { opacity: .65; }
  .tab-target {
    display: flex;
    min-width: 0;
    height: 100%;
    flex: 1;
    align-items: center;
    gap: 8px;
    padding: 0 4px 0 12px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-md);
    text-align: left;
  }
  .notebook-tab.active .tab-target { color: var(--text); font-weight: 600; }
  .tab-target:hover:not(:disabled) { color: var(--text); }
  .tab-target:disabled, .tab-close:disabled { cursor: default; opacity: var(--disabled); }
  .tab-target:focus-visible, .tab-close:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: -2px; }
  .tab-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tab-close {
    display: grid;
    width: var(--control-dense);
    height: var(--control-dense);
    margin-right: 2px;
    flex: none;
    padding: 0;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    place-items: center;
  }
  .tab-close svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: var(--stroke-dense);
  }
  .tab-close:hover:not(:disabled) { background: var(--wash); color: var(--text); }
  .tab-close:active:not(:disabled) { background: var(--wash-strong); }
  .notebook-tab.active .tab-close:hover:not(:disabled) { background: var(--wash-strong); }
  .state-dot {
    width: 6px;
    height: 6px;
    flex: none;
    border-radius: 50%;
    background: var(--blueprint-light);
  }
  /* Real work in progress: opacity only, and it stops when saving stops. */
  .state-dot.saving { animation: breathe 1s ease-in-out infinite alternate; }
  .state-dot.warning { background: var(--oxide); }
  @keyframes breathe { from { opacity: .35; } to { opacity: 1; } }
  @media (max-width: 800px) {
    .notebook-tab { min-width: 118px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .notebook-tab { transition: none; }
    .state-dot.saving { animation: none; }
  }
</style>
