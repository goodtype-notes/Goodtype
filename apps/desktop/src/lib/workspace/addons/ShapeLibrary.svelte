<script lang="ts">
  import { onMount } from "svelte";
  import { displayLocale, formatNumber, locale, t } from "../../i18n";
  import PackShapePreview from "./PackShapePreview.svelte";
  import { searchShapes, shapeLibraries, type ContentPack, type ShapeChoice } from "./content";

  let { packs, color, width, onInsert, onClose }: {
    packs: ContentPack[];
    color: string;
    width: number;
    onInsert: (item: ShapeChoice, scale: number) => Promise<void>;
    onClose: () => void;
  } = $props();

  let sheet: HTMLDialogElement;
  let searchField: HTMLInputElement;
  let library = $state("");
  let query = $state("");
  let selection = $state<ShapeChoice>();
  let scale = $state(1);
  let busy = $state(false);
  let failed = $state(false);
  const libraries = $derived(shapeLibraries(packs, $locale));
  const items = $derived(searchShapes(libraries, library, query, $locale));
  const selected = $derived(items.find(({ choice }) =>
    choice.addonId === selection?.addonId && choice.libraryId === selection?.libraryId && choice.itemId === selection?.itemId,
  ) ?? items[0]);

  onMount(() => {
    sheet.showModal();
    searchField.focus();
    return () => sheet.close();
  });

  async function insert() {
    if (!selected || busy) return;
    busy = true;
    failed = false;
    try {
      await onInsert(selected.choice, scale);
      onClose();
    } catch {
      failed = true;
    } finally {
      busy = false;
    }
  }
</script>

<dialog bind:this={sheet} aria-labelledby="shape-library-title" onkeydown={event => {
  if (event.key !== "Escape") return;
  event.stopPropagation();
  event.preventDefault();
  if (!busy) onClose();
}} oncancel={event => {
  event.preventDefault();
  if (!busy) onClose();
}}>
  <header>
    <h2 id="shape-library-title">{$t("contentPacks.insertShape")}</h2>
    <button class="close" type="button" aria-label={$t("library.cancel")} onclick={onClose} disabled={busy}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
    </button>
  </header>
  <div class="body">
    <section class="library">
      <input type="search" aria-label={$t("contentPacks.search")} placeholder={$t("contentPacks.search")} bind:value={query} bind:this={searchField} />
      <div class="chips">
        <button type="button" aria-pressed={!library} onclick={() => library = ""}>{$t("contentPacks.all")}</button>
        {#each libraries as shelf (shelf.id)}
          <button type="button" aria-pressed={shelf.id === library} onclick={() => library = shelf.id}>{shelf.label}</button>
        {/each}
      </div>
      {#if items.length}
        <div class="grid">
          {#each items as row (JSON.stringify(row.choice))}
            <button class="shape" type="button" aria-pressed={selected === row} onclick={() => selection = row.choice}>
              <span class="thumbnail"><PackShapePreview item={row.item} {color} {width} /></span>
              <span>{row.label}</span>
            </button>
          {/each}
        </div>
      {:else}
        <p class="empty">{$t(libraries.length ? "contentPacks.noShapesFound" : "contentPacks.noShapes")}</p>
      {/if}
    </section>
    <aside>
      <h3>{$t("contentPacks.preview")}</h3>
      {#if selected}
        <div class="preview"><PackShapePreview item={selected.item} {color} {width} /></div>
        <strong>{selected.label}</strong>
        <label>
          {$t("contentPacks.scale")}
          <select bind:value={scale} disabled={busy}>
            {#each [0.25, 0.5, 1, 1.5, 2, 3, 4, 6, 8] as size}
              <option value={size}>{formatNumber(size, { style: "percent" }, $displayLocale)}</option>
            {/each}
          </select>
        </label>
      {/if}
    </aside>
  </div>
  <footer>
    {#if failed}<span role="alert">{$t("contentPacks.insertFailed")}</span>{/if}
    <button class="secondary" type="button" onclick={onClose} disabled={busy}>{$t("library.cancel")}</button>
    <button class="primary" type="button" onclick={() => void insert()} disabled={busy || !selected}>{$t("contentPacks.insert")}</button>
  </footer>
</dialog>

<style>
  dialog { width: min(760px, calc(100vw - 48px)); max-height: calc(100dvh - 48px); padding: 0; color: var(--text); background: var(--panel); border: 1px solid var(--edge); border-radius: var(--radius-lg); box-shadow: var(--shadow-modal); font: var(--text-md) var(--font-ui); }
  dialog[open] { display: flex; flex-direction: column; }
  dialog::backdrop { background: rgb(0 0 0 / 45%); }
  header, footer { display: flex; align-items: center; flex: none; gap: 8px; padding: 14px 20px; }
  header { justify-content: space-between; border-bottom: 1px solid var(--edge-soft); }
  h2, h3, p { margin: 0; }
  h2 { font-size: var(--text-lg); font-weight: 600; }
  h3 { font-size: var(--text-sm); font-weight: 600; color: var(--muted); }
  .body { display: grid; grid-template-columns: minmax(0, 1fr) minmax(170px, 28%); min-height: 0; overflow: auto; }
  .library, aside { padding: 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  aside { border-left: 1px solid var(--edge-soft); }
  .chips, .grid { display: flex; flex-wrap: wrap; gap: 6px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(116px, 1fr)); align-content: start; min-height: min(210px, 40dvh); max-height: min(360px, 48dvh); overflow: auto; padding: 2px; margin: -2px; }
  .shape { display: grid; gap: 6px; grid-template-rows: 54px auto; padding: 7px; background: var(--fill-soft); text-align: left; overflow-wrap: anywhere; }
  .thumbnail, .preview { display: block; padding: 6px; background: var(--paper); border-radius: var(--radius); }
  .preview { height: 110px; }
  .empty { color: var(--muted); min-height: 210px; }
  button, input, select { font: inherit; color: inherit; border: 0; border-radius: var(--radius); }
  button { min-height: var(--control-dense); background: transparent; cursor: pointer; }
  button:hover:not(:disabled) { background: var(--wash); }
  button[aria-pressed="true"] { background: var(--accent-fill); box-shadow: inset 0 0 0 1px var(--accent-edge); }
  button:disabled { opacity: var(--disabled); cursor: not-allowed; }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  .close { display: grid; place-items: center; width: var(--control-dense); color: var(--muted); }
  input, select { height: var(--control); border: 1px solid var(--edge); padding: 0 8px; background: var(--inset); color-scheme: dark; }
  label { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
  footer { justify-content: end; border-top: 1px solid var(--edge-soft); }
  footer span { color: var(--oxide-text); margin-right: auto; }
  footer button { min-height: var(--control); padding: 0 14px; font-weight: 600; }
  .secondary { background: var(--wash); }
  .secondary:hover:not(:disabled) { background: var(--wash-strong); }
  .primary { background: var(--blueprint); color: var(--on-accent); }
  .primary:hover:not(:disabled) { background: var(--blueprint-hover); }
</style>
