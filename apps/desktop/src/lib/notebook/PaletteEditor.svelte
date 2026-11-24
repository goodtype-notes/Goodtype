<script lang="ts">
  import { t } from "../i18n";
  import { type Snippet } from "svelte";
  import type { PaletteDock } from "./palette";
  import { placePalettePopup } from "./palettePopup";

  let { title, anchor, dock, onClose, onDismiss, children }: {
    title: string;
    anchor: HTMLElement;
    dock: PaletteDock;
    onClose: () => void;
    onDismiss: () => void;
    children: Snippet;
  } = $props();

</script>

<div use:placePalettePopup={{ anchor, dock, onDismiss, focus: "[aria-pressed='true'], input, button" }} class="palette-editor" data-ui="solid" role="dialog" aria-label={title} tabindex="-1"
  onkeydown={(event) => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } }}>
  <header><strong>{title}</strong><button type="button" aria-label={$t("workspace.close2", { "value1": title.toLowerCase() })} onclick={onClose}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
  </button></header>
  {@render children()}
</div>

<style>
  .palette-editor { position: fixed; z-index: 60; width: 272px; max-width: calc(100vw - 32px); max-height: calc(100vh - 140px); overflow-y: auto; border: 1px solid var(--edge); border-radius: var(--radius-lg); background: var(--panel); color: var(--text); box-shadow: var(--shadow-panel); }
  header { display: flex; align-items: center; justify-content: space-between; min-height: var(--control); padding: 0 6px 0 12px; border-bottom: 1px solid var(--edge-soft); }
  strong { font-size: var(--text-lg); line-height: 1.25; font-weight: 600; }
  button { display: grid; place-items: center; width: var(--control-dense); height: var(--control-dense); border: 0; border-radius: var(--radius); background: transparent; color: var(--muted); cursor: pointer; }
  button:hover { background: var(--wash); color: var(--text); }
  button:active { background: var(--wash-strong); }
  button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  svg { width: var(--icon-dense); height: var(--icon-dense); fill: none; stroke: currentColor; stroke-width: var(--stroke-dense); stroke-linecap: round; }
</style>
