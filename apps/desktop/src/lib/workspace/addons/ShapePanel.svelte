<script lang="ts">
  import { onMount } from "svelte";
  import { locale, renderMessage, t, type UiText } from "../../i18n";
  import PackShapePreview from "./PackShapePreview.svelte";
  import { searchShapes, type ShapeChoice, type ShapeLibraryModel } from "./content";
  import { dragStarts, recentRows, sameShape, shapeKey, type ShapeDrag, type ShapeRow as Row } from "./shapePanel";

  let { libraries, recent, stamp, failure = null, dock, onStamp, onPlace, onDrag, onClose }: {
    libraries: ShapeLibraryModel;
    recent: string[];
    stamp: ShapeChoice | null;
    failure?: UiText | null;
    dock: "left" | "right";
    onStamp: (choice: ShapeChoice) => void;
    /** Keyboard placement at the centre of the visible page. */
    onPlace: (choice: ShapeChoice) => void;
    onDrag: (drag: ShapeDrag) => void;
    onClose: () => void;
  } = $props();

  let searchField: HTMLInputElement;
  let list: HTMLElement;
  let query = $state("");
  let collapsed = $state<Record<string, boolean>>({});
  const searching = $derived(query.trim().length > 0);
  const recentItems = $derived(searching ? [] : recentRows(libraries, recent));
  const groups = $derived(libraries.map(library => ({
    library,
    items: searching ? searchShapes([library], "", query, $locale) : library.items,
  })).filter(group => group.items.length > 0));

  onMount(() => {
    searchField.focus();
    // Once a finger has turned into a drag the list must stop scrolling under it. Pointer events
    // reach the handlers first, so a non-passive touchmove can cancel the pan that follows.
    const holdScroll = (event: TouchEvent) => { if (press?.dragging) event.preventDefault(); };
    list.addEventListener("touchmove", holdScroll, { passive: false });
    return () => list.removeEventListener("touchmove", holdScroll);
  });

  // A press only becomes a drag once it has clearly left the tile (see `dragStarts`), so a finger
  // can still scroll the list and a tap still toggles stamping.
  let press: { pointerId: number; type: string; x: number; y: number; at: number; row: Row; dragging: boolean } | null = null;
  let swallowClick = false;

  function pressTile(event: PointerEvent, row: Row) {
    if (event.button !== 0 && event.pointerType !== "touch" && event.pointerType !== "pen") return;
    const tile = event.currentTarget as HTMLElement;
    press = { pointerId: event.pointerId, type: event.pointerType, x: event.clientX, y: event.clientY, at: event.timeStamp, row, dragging: false };
    // Captured from the start so a quick flick that leaves the tile before its first move still
    // reaches it. A finger is captured implicitly, and the browser still takes it for a scroll.
    tile.setPointerCapture(event.pointerId);
  }

  function moveTile(event: PointerEvent) {
    if (press?.pointerId !== event.pointerId) return;
    if (!press.dragging) {
      if (!dragStarts(press.type, event.clientX - press.x, event.clientY - press.y, event.timeStamp - press.at)) return;
      press.dragging = true;
    }
    event.preventDefault();
    onDrag({ phase: "move", row: press.row, x: event.clientX, y: event.clientY, alt: event.altKey });
  }

  function releaseTile(event: PointerEvent) {
    if (press?.pointerId !== event.pointerId) return;
    const done = press;
    press = null;
    if (!done.dragging) return;
    swallowClick = true;
    setTimeout(() => (swallowClick = false), 0);
    onDrag({ phase: event.type === "pointerup" ? "drop" : "cancel", row: done.row, x: event.clientX, y: event.clientY, alt: event.altKey });
  }

  function clickTile(row: Row) {
    if (swallowClick) return;
    onStamp(row.choice);
  }

  function keyTile(event: KeyboardEvent, row: Row) {
    if (event.key !== "Enter") return;
    event.preventDefault();
    onPlace(row.choice);
  }
</script>

{#snippet tiles(rows: Row[], group: string)}
  <div class="grid">
    {#each rows as row (group + shapeKey(row.choice))}
      <button class="tile" type="button" title={row.label} aria-pressed={sameShape(stamp, row.choice)}
        onpointerdown={event => pressTile(event, row)} onpointermove={moveTile}
        onpointerup={releaseTile} onpointercancel={releaseTile}
        onclick={() => clickTile(row)} onkeydown={event => keyTile(event, row)}
        oncontextmenu={event => event.preventDefault()}>
        <span class="drawing"><PackShapePreview item={row.item} color="currentColor" width={1.4} /></span>
        <span class="label">{row.label}</span>
      </button>
    {/each}
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside class="shape-panel" class:dock-left={dock === "left"} aria-labelledby="shape-panel-title" data-preserve-selection
  onkeydown={event => {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    event.preventDefault();
    if (stamp) onStamp(stamp);
    else onClose();
  }}>
  <header>
    <h2 id="shape-panel-title">{$t("contentPacks.shapes")}</h2>
    <button class="close" type="button" aria-label={$t("contentPacks.closeShapes")} title={$t("contentPacks.closeShapes")} onclick={onClose}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
    </button>
  </header>
  <input type="search" aria-label={$t("contentPacks.search")} placeholder={$t("contentPacks.search")} bind:value={query} bind:this={searchField} />
  <p class="hint" class:stamping={Boolean(stamp)} role="status">
    {#if failure}
      <span class="error">{renderMessage(failure, $t)}</span>
    {:else}
      {$t(stamp ? "contentPacks.stampHint" : "contentPacks.placeHint")}
    {/if}
  </p>
  <div class="list" bind:this={list}>
    {#if recentItems.length}
      <section aria-labelledby="shape-panel-recent">
        <h3 id="shape-panel-recent" class="section-label">{$t("contentPacks.recent")}</h3>
        {@render tiles(recentItems, "recent:")}
      </section>
    {/if}
    {#each groups as { library, items } (library.id)}
      {@const open = searching || !collapsed[library.id]}
      <section>
        <h3>
          <button class="library" type="button" aria-expanded={open} disabled={searching}
            onclick={() => (collapsed = { ...collapsed, [library.id]: open })}>
            <svg class:open viewBox="0 0 24 24" aria-hidden="true"><path d="m9 6 6 6-6 6" /></svg>
            <span>{library.label}</span>
          </button>
        </h3>
        {#if open}{@render tiles(items, library.id)}{/if}
      </section>
    {:else}
      <p class="empty">{$t("contentPacks.noShapesMatch", { query: query.trim() })}</p>
    {/each}
  </div>
</aside>

<style>
  .shape-panel {
    position: absolute;
    z-index: 30;
    top: var(--shape-panel-top, 12px);
    right: 12px;
    bottom: var(--shape-panel-bottom, 12px);
    display: flex;
    width: 288px;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-panel);
    font: var(--text-md) var(--font-ui);
    animation: enter var(--enter) ease-out;
  }
  .shape-panel.dock-left { right: auto; left: 12px; }
  @keyframes enter { from { opacity: 0; transform: translateY(4px); } }
  @media (prefers-reduced-motion: reduce) { .shape-panel { animation: none; } }

  header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  h2, h3, p { margin: 0; }
  h2 { font-size: var(--text-lg); font-weight: 600; }
  .section-label { padding: 2px 4px 6px; color: var(--text); font-size: var(--text-md); font-weight: 600; }

  button, input { font: inherit; color: inherit; border: 0; border-radius: var(--radius); }
  button { background: transparent; cursor: pointer; transition: background var(--fast) ease, color var(--fast) ease; }
  button:active:not(:disabled) { background: var(--wash-strong); }
  button:hover:not(:disabled) { background: var(--wash); }
  button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  button:disabled { cursor: default; }
  svg { width: var(--icon-dense); height: var(--icon-dense); flex: none; fill: none; stroke: currentColor; stroke-width: var(--stroke-dense); stroke-linecap: round; stroke-linejoin: round; }

  .close { display: grid; place-items: center; width: var(--control-dense); height: var(--control-dense); color: var(--muted); }
  .close:hover { color: var(--text); }

  input { height: var(--control); flex: none; padding: 0 12px; background: var(--inset); border: 1px solid var(--edge); color-scheme: dark; }
  input::placeholder { color: var(--quiet); }
  input:hover { border-color: var(--edge-strong); }
  input:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }

  .hint { min-height: 1lh; color: var(--muted); font-size: var(--text-sm); }
  .hint.stamping { color: var(--text); }
  .error { color: var(--oxide-text); }

  .list { display: flex; min-height: 0; flex: 1; flex-direction: column; gap: 6px; margin: 0 -12px -12px; padding: 0 12px 12px; overflow-y: auto; overscroll-behavior: contain; }
  .library { display: flex; width: 100%; min-height: var(--control-dense); align-items: center; gap: 4px; padding: 0 4px; color: var(--text); font-size: var(--text-md); font-weight: 600; text-align: left; }
  .library svg { transition: transform var(--fast) ease; }
  .library svg.open { transform: rotate(90deg); }
  .library span:first-of-type { flex: 1; }

  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px; padding-top: 2px; }
  .tile { display: grid; grid-template-rows: 44px auto; gap: 4px; min-width: 0; padding: 6px 4px; color: var(--text); text-align: center; touch-action: pan-y; user-select: none; -webkit-user-select: none; }
  .tile[aria-pressed="true"] { background: var(--accent-fill); box-shadow: inset 0 0 0 1px var(--accent-edge); }
  .tile[aria-pressed="true"]:hover { background: var(--accent-fill); }
  .tile:active:not([aria-pressed="true"]) { background: var(--wash-strong); }
  .drawing { display: block; min-width: 0; pointer-events: none; }
  .label { overflow: hidden; color: var(--muted); font-size: var(--text-sm); line-height: 1.25; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow-wrap: break-word; hyphens: auto; }
  .tile:hover .label, .tile[aria-pressed="true"] .label { color: var(--text); }
  .tile[aria-pressed="true"] .label { font-weight: 600; }
  .empty { padding: 8px 4px; color: var(--muted); }

  /* At narrow windows the panel keeps most of the page visible: two columns instead of three. */
  @media (max-width: 1000px) {
    .shape-panel { width: 212px; }
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
</style>
