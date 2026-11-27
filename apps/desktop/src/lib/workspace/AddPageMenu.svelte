<script lang="ts">
  import type { Snippet } from "svelte";
  import { t, formatNumber, displayLocale } from "../i18n";
  import { dismissable } from "./dismiss";
  import { sourceBehavior, resolvePosition, type AddPageGroup, type AddPageSource, type AddPageWhere } from "./addPage";
  import { isPageListSyntax } from "./exportTarget";

  /**
   * The add-page panel: three columns instead of one long scroll.
   *
   * Settings on the left, the paper library in the middle, and a preview on the right showing the
   * page the button will actually make. The old menu stacked every decision in one 348px column,
   * so the templates sat four scrolls below the size they were drawn at and the writer never saw
   * the result until the page existed.
   *
   * The change that makes the rest possible: picking a paper no longer *makes* a page. A template
   * is a selection and the Add button is the commit. That is what lets a preview exist, what lets
   * arrow keys walk the library without creating six pages on the way past, and what lets a run
   * of pages be asked for once rather than by reopening this panel.
   *
   * Position and paper are remembered by the caller rather than reset each time it opens.
   */
  let {
    editBackground = false,
    trigger,
    generators,
    where,
    groups,
    tones,
    toneId,
    sizes,
    sizeId,
    orientation,
    previewAspect,
    geometry,
    currentPageId,
    pageNumber,
    pageCount,
    canPlaceRelative,
    onWhereChange,
    onToneChange,
    onSizeChange,
    onOrientationChange,
    onClose,
  }: {
    editBackground?: boolean;
    trigger?: HTMLElement;
    generators?: Snippet<[import("../model").PagePosition]>;
    where: AddPageWhere;
    groups: AddPageGroup[];
    /** Paper colours. Every template below is shown in whichever one is selected. */
    tones: { id: string; name: string; backgroundColor: string }[];
    toneId: string;
    /** Page sizes, portrait. Landscape is the orientation toggle, not separate entries. */
    sizes: { id: string; name: string; detail: string }[];
    sizeId: string;
    orientation: "portrait" | "landscape";
    /** Width over height of the size being previewed, so a swatch is never a distorted page. */
    previewAspect: number;
    /** The chosen size after orientation, so the panel can read its own dimensions back. */
    geometry: { widthPt: number; heightPt: number };
    /** The page "before" and "after" are relative to. */
    currentPageId: string;
    /** One-based, as the writer sees it. */
    pageNumber: number;
    pageCount: number;
    /** False when no page is open, which leaves appending as the only meaning "add" can have. */
    canPlaceRelative: boolean;
    onWhereChange: (next: AddPageWhere) => void;
    onToneChange: (next: string) => void;
    onSizeChange: (next: string) => void;
    onOrientationChange: (next: "portrait" | "landscape") => void;
    onClose: () => void;
  } = $props();

  const CHOICES: { value: AddPageWhere; label: string }[] = $derived([
    { value: "before", label: $t("workspace.before") },
    { value: "after", label: $t("workspace.after") },
    { value: "last", label: $t("workspace.lastPage") },
  ]);

  const ORIENTATIONS = $derived([
    { value: "portrait" as const, label: $t("library.portrait") },
    { value: "landscape" as const, label: $t("library.landscape") },
  ]);

  const MM_PER_PT = 25.4 / 72;
  const MAX_RUN = 50;

  let category = $state("all");
  let selectedId = $state<string | null>(null);
  let count = $state(1);
  let added = $state(0);
  let panel = $state<HTMLElement>();
  /** Which PDF pages to bring in. Only the syntax is checked here; the file is not chosen yet. */
  let pdfPages = $state("");

  /** Paper shelves. The import shelf is pinned above the library rather than scrolling with it. */
  const paperGroups = $derived(groups.filter((group) => (group.lane ?? "blank") === "blank"));
  const importSources = $derived(
    groups.filter((group) => group.lane === "import").flatMap((group) => group.sources),
  );
  /// The page you are on. A starting point rather than a kind of ruling, so it is pinned beside
  /// the import instead of standing among Plain / Lines / Squares / Dots as though it were one.
  const currentSources = $derived(
    groups.filter((group) => group.lane === "current").flatMap((group) => group.sources),
  );
  const pinned = $derived([...currentSources, ...importSources]);
  const everySource = $derived([...pinned, ...paperGroups.flatMap((group) => group.sources)]);

  /// Opens on the notebook's paper: the one it was created with, then whatever was added last.
  const selected = $derived<AddPageSource | undefined>(
    everySource.find((source) => source.id === selectedId) ??
      everySource.find((source) => source.id === "next") ??
      everySource[0],
  );
  const behavior = $derived(sourceBehavior(selected));
  const importing = $derived(behavior.importFile);
  const effectiveCount = $derived(behavior.single ? 1 : count);
  const pdfPagesValid = $derived(!behavior.pdf || isPageListSyntax(pdfPages));
  const blocked = $derived(!selected || selected.disabled || !pdfPagesValid);

  const shelves = $derived(
    category === "all" ? paperGroups : paperGroups.filter((group) => group.id === category),
  );

  const readout = $derived.by(() => {
    if (behavior.calculation) return $t("calculation.geometry");
    const inches = sizes.find((size) => size.id === sizeId)?.detail.includes("in");
    const width = geometry.widthPt * MM_PER_PT;
    const height = geometry.heightPt * MM_PER_PT;
    return inches
      ? $t("workspace.in", { "value1": formatNumber(width / 25.4, { minimumFractionDigits: 1, maximumFractionDigits: 1 }, $displayLocale), "value2": formatNumber(height / 25.4, { minimumFractionDigits: 1, maximumFractionDigits: 1 }, $displayLocale) })
      : $t("workspace.mm3", { "value1": Math.round(width), "value2": Math.round(height) });
  });

  // Reads back what the choice actually means for this notebook, so "Before" on page 1 says so
  // rather than leaving the writer to work out where the page will appear.
  const destination = $derived(
    !canPlaceRelative || where === "last"
      ? pageCount > 0
        ? $t("workspace.newPageAtTheEnd", { "value1": pageCount + 1 })
        : $t("workspace.theFirstPageOfTheNotebook")
      : where === "before"
        ? $t("workspace.newPagePushingPageDown", { "pageNumber": pageNumber, "pageNumber2": pageNumber })
        : $t("workspace.newPageAfterThePageYouAre", { "value1": pageNumber + 1 }),
  );

  const addLabel = $derived(
    editBackground ? $t("workspace.applyBackground") : behavior.pdf ? $t("workspace.importPDF")
      : behavior.kind === "calculation-import" ? $t("calculation.import")
      : behavior.calculation ? (selected?.id === "calculation-part" ? $t("calculation.continuePage") : $t("calculation.addPage"))
      : count > 1 ? $t("workspace.addPages", { count }) : $t("workspace.addPage"),
  );

  const summary = $derived([
    {
      key: "size",
      label: $t("workspace.size"),
      value: editBackground ? readout : behavior.calculation ? $t("calculation.geometry") : behavior.pdf ? $t("calculation.fromPDF") : `${sizes.find((size) => size.id === sizeId)?.name ?? ""} ${orientation === "portrait" ? $t("library.portrait") : $t("library.landscape")} · ${readout}`,
    },
    { key: "paper", label: $t("workspace.paper"), value: behavior.paper ? tones.find((tone) => tone.id === toneId)?.name ?? "" : behavior.calculation ? $t("calculation.cellContent") : $t("calculation.fromPDF") },
    { key: "ruling", label: $t("calculation.source"), value: behavior.pdf ? $t("calculation.fromPDF") : (selected?.label ?? "") },
    { key: "where", label: $t("workspace.where"), value: CHOICES.find(choice => choice.value === (canPlaceRelative ? where : "last"))?.label ?? "" },
  ].filter(row => !editBackground || row.key !== "where"));

  function commit(keepOpen: boolean) {
    if (!selected || blocked) return;
    selected.onSelect(
      resolvePosition(canPlaceRelative ? where : "last", currentPageId),
      effectiveCount,
      behavior.pdf ? { pdfPages: pdfPages.trim() } : undefined,
    );
    if (keepOpen && !editBackground) added += effectiveCount;
    else onClose();
  }

  function step(by: number) {
    count = Math.max(1, Math.min(MAX_RUN, count + by));
  }

  /**
   * Arrow keys inside one group.
   *
   * Every group here selects rather than commits, so landing on something is safe: arrowing
   * across the library previews each paper in turn, which is the point of having a preview.
   */
  function moveWithin(event: KeyboardEvent, columns = 0) {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
    // Bound to the buttons rather than the group: a radiogroup that is itself focusable puts a
    // stop in the tab order that selects nothing.
    const group = (event.currentTarget as HTMLElement).closest<HTMLElement>(
      "[role='radiogroup'], [role='group']",
    );
    const items = group ? [...group.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")] : [];
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    if (current < 0) return;
    const stride = columns > 0 ? columns : 1;
    let next = current;
    if (event.key === "ArrowRight") next = current + 1;
    else if (event.key === "ArrowLeft") next = current - 1;
    else if (event.key === "ArrowDown") next = current + stride;
    else if (event.key === "ArrowUp") next = current - stride;
    else if (event.key === "Home") next = 0;
    else next = items.length - 1;
    next = Math.max(0, Math.min(items.length - 1, next));
    event.preventDefault();
    if (next === current) return;
    items[next].focus();
    items[next].click();
  }

  /// Enter commits from wherever the writer is, and Tab stays inside the panel so a keyboard
  /// writer cannot land behind it on the page it is covering.
  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !(event.target as HTMLElement).closest(".add, .generators")) {
      event.preventDefault();
      commit(event.shiftKey);
      return;
    }
    if (event.key !== "Tab" || !panel) return;
    const stops = [...panel.querySelectorAll<HTMLElement>("button:not(:disabled)")].filter(
      (stop) => stop.tabIndex >= 0,
    );
    if (stops.length < 2) return;
    const first = stops[0];
    const last = stops[stops.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  /// Opens on the commit: the panel already carries a sensible page, so Enter is a complete
  /// answer and everything else is an adjustment to it.
  $effect(() => {
    panel?.querySelector<HTMLElement>(".add")?.focus();
  });
</script>

<div
  bind:this={panel}
 
  use:dismissable={{ onClose, trigger }}
  class="add-page-panel"
  role="dialog"
  tabindex="-1"
  aria-label={$t(editBackground ? "workspace.pageBackground" : "workspace.addPage")}
  onkeydown={keydown}
>
  <header>
    <div class="subject">
      <strong>{$t(editBackground ? "workspace.pageBackground" : "workspace.addPage")}</strong>
      {#if !editBackground}<span class="destination">{destination}</span>{/if}
    </div>

    {#if !editBackground}<div class="run" role="group" aria-label={$t("workspace.howManyPages")}>
      <span class="run-label">{$t("workspace.pages")}</span>
      <button type="button" aria-label={$t("workspace.oneFewerPage")} disabled={behavior.single || count <= 1} onclick={() => step(-1)}>−</button>
      <output aria-live="polite">{effectiveCount}</output>
      <button type="button" aria-label={$t("workspace.oneMorePage")} disabled={behavior.single || count >= MAX_RUN} onclick={() => step(1)}>+</button>
    </div>

    {/if}
    <button type="button" class="close" aria-label={$t("library.close")} onclick={onClose}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
    </button>
  </header>

  <div class="body">
    <!-- Column A. Everything that decides the shape of the page, visible at once. -->
    <div class="settings">
      {#if !editBackground}<section>
        <div class="eyebrow">{$t("workspace.where")}</div>
        <div class="row" role="radiogroup" aria-label={$t("workspace.whereTheNewPageGoes")}>
          {#each CHOICES as choice (choice.value)}
            {@const on = (canPlaceRelative ? where : "last") === choice.value}
            <button
              type="button"
              role="radio"
              class="chip"
              class:on
              disabled={!canPlaceRelative && choice.value !== "last"}
              aria-checked={on}
              tabindex={on ? 0 : -1}
              onkeydown={(event) => moveWithin(event)}
              onclick={() => onWhereChange(choice.value)}
            >{choice.label}</button>
          {/each}
        </div>
      </section>

      <section inert={!behavior.paper} class:muted={!behavior.paper}>
        <div class="eyebrow spread">
          <span>{$t("workspace.size")}</span>
          <span class="readout">{readout}</span>
        </div>
        <div class="pairs" role="radiogroup" aria-label={$t("library.pageSize")}>
          {#each sizes as size (size.id)}
            <button
              type="button"
              role="radio"
              class="chip"
              class:on={size.id === sizeId}
              aria-checked={size.id === sizeId}
              tabindex={size.id === sizeId ? 0 : -1}
              title={size.detail}
              onkeydown={(event) => moveWithin(event, 2)}
              onclick={() => onSizeChange(size.id)}
            >{size.name}</button>
          {/each}
        </div>
      </section>

      <div class="row" inert={!behavior.paper} class:muted={!behavior.paper} role="radiogroup" aria-label={$t("library.orientation")}>
        {#each ORIENTATIONS as choice (choice.value)}
          <button
            type="button"
            role="radio"
            class="chip"
            class:on={orientation === choice.value}
            aria-checked={orientation === choice.value}
            tabindex={orientation === choice.value ? 0 : -1}
            onkeydown={(event) => moveWithin(event)}
            onclick={() => onOrientationChange(choice.value)}
          >
            <!-- The shape is the label: a page turned on its side says it faster than a word. -->
            <span class="page-mark" class:landscape={choice.value === "landscape"} aria-hidden="true"></span>
            {choice.label}
          </button>
        {/each}
      </div>

      {/if}
      <section inert={!behavior.paper} class:muted={!behavior.paper}>
        <div class="eyebrow">{$t("workspace.paper")}</div>
        <div class="row" role="radiogroup" aria-label={$t("library.paperColour")}>
          {#each tones as tone (tone.id)}
            <button
              type="button"
              role="radio"
              class="chip tone"
              class:on={tone.id === toneId}
              aria-checked={tone.id === toneId}
              tabindex={tone.id === toneId ? 0 : -1}
              onkeydown={(event) => moveWithin(event)}
              onclick={() => onToneChange(tone.id)}
            >
              <span class="swatch" style:background={tone.backgroundColor} aria-hidden="true"></span>
              {tone.name}
            </button>
          {/each}
        </div>
      </section>

      {#if behavior.pdf}
        <!-- The one setting a PDF does read: which of its pages to bring in. -->
        <section class="pdf-pages">
          <label class="eyebrow" for="add-page-pdf-pages">{$t("workspace.pdfPageList")}</label>
          <input
            id="add-page-pdf-pages"
            type="text"
            inputmode="numeric"
            spellcheck="false"
            autocomplete="off"
            bind:value={pdfPages}
            placeholder={$t("workspace.pdfPageListPlaceholder")}
            aria-invalid={!pdfPagesValid}
            aria-describedby="add-page-pdf-pages-help"
          />
          <span id="add-page-pdf-pages-help" class="help" class:invalid={!pdfPagesValid}>
            {pdfPagesValid ? $t("workspace.pdfPageListHelp") : $t("workspace.pdfPageListInvalid")}
          </span>
        </section>
      {/if}
    </div>

    <!-- Column B. Filters and the import sit in a fixed band; only the papers scroll. -->
    <div class="library">
      <div class="band">
        {#if generators}
          <div class="generators">{@render generators(resolvePosition(canPlaceRelative ? where : "last", currentPageId))}</div>
        {/if}
        <div class="filters" role="group" aria-label={$t("workspace.paperKind")}>
          <button
            type="button"
            class="pill"
            class:on={category === "all"}
            aria-pressed={category === "all"}
            onclick={() => (category = "all")}
          >{$t("workspace.all")}</button>
          {#each paperGroups as group (group.id)}
            <button
              type="button"
              class="pill"
              class:on={category === group.id}
              aria-pressed={category === group.id}
              onclick={() => (category = group.id)}
            >{group.title}</button>
          {/each}
        </div>

        <!-- Pinned above the library rather than filed inside it. Neither the page you are on nor
             a document is a kind of ruling, and both used to sit among the paper shelves. -->
        <div class="pins" role="group" aria-label={$t("workspace.startFrom")}>
        {#each pinned as source (source.id)}
          <button
            type="button"
            class="import"
            class:on={selected?.id === source.id}
            disabled={source.disabled}
            aria-pressed={selected?.id === source.id}
            title={source.detail ? `${source.label} · ${source.detail}` : source.label}
            onkeydown={(event) => moveWithin(event)}
            onclick={() => (selectedId = source.id)}
          >
            <span class="sheet" aria-hidden="true">
              {#if sourceBehavior(source).pdf}
                <span class="badge">PDF</span>
              {:else if sourceBehavior(source).calculation}
                <svg viewBox="0 0 48 56" fill="none" aria-hidden="true"><path d="M7 4h34v48H7z" stroke="#7a91a0"/><rect x="12" y="12" width="24" height="12" rx="2" stroke="#4e8297"/><path d="m19 15-3 3 3 3m10-6 3 3-3 3M13 32h22M13 38h17" stroke="#4e8297" stroke-width="2"/></svg>
              {:else if source.preview}
                {@html source.preview}
              {/if}
            </span>
            <span class="words">
              <span class="title">{source.label}</span>
              <span class="sub">{source.disabled ? $t("workspace.needsTheDesktopApp") : (source.detail ?? "")}</span>
            </span>
          </button>
        {/each}
        </div>
      </div>

      <!-- Dimmed while the PDF is selected: the file brings its own pages, so none of this
           applies to what the button is about to do. -->
      <div class="papers" class:muted={!behavior.paper}>
        {#each shelves as group (group.id)}
          <div class="shelf">
            <div class="eyebrow">{group.title}</div>
            <div class="cards" role="group" aria-label={group.title}>
              {#each group.sources as source (source.id)}
                <button
                  type="button"
                  class="card"
                  class:on={selected?.id === source.id}
                  disabled={source.disabled}
                  aria-pressed={selected?.id === source.id}
                  onkeydown={(event) => moveWithin(event, 4)}
                  onclick={() => (selectedId = source.id)}
                >
                  <span class="thumb" style:aspect-ratio={previewAspect} aria-hidden="true">
                    {#if source.preview}
                      <!-- Built into this app, never read from an imported file. -->
                      {@html source.preview}
                    {/if}
                  </span>
                  <span class="name">{source.label}</span>
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    </div>

    <!-- Column C. Not a swatch of the paper: the page, at the proportions it will have. -->
    <div class="preview">
      <div class="eyebrow">{$t("workspace.preview")}</div>
      <div class="stage">
        <div class="sheet-preview" class:pdf={behavior.pdf} style:aspect-ratio={behavior.calculation ? 1.45 : previewAspect} style:--ratio={behavior.calculation ? 1.45 : previewAspect}>
          {#if behavior.pdf}
            <span class="badge big">PDF</span>
          {:else if behavior.calculation}
            <div class="calculation-preview" aria-hidden="true"><span class="code-line"></span><span class="code-line short"></span><svg viewBox="0 0 100 45"><path d="M5 4v35h90" stroke="#aebac2" fill="none"/><path d="M8 28C24 2 35 1 47 23s28 24 45-10" stroke="#4e8297" stroke-width="2" fill="none"/></svg></div>
          {:else if selected?.preview}
            {@html selected.preview}
          {/if}
        </div>
      </div>

      <dl class="summary">
        {#each summary as row (row.key)}
          <div>
            <dt>{row.label}</dt>
            <dd>{row.value}</dd>
          </div>
        {/each}
      </dl>

      <div class="commit">
        <button
          type="button"
          class="add"
          disabled={blocked}
          onclick={(event) => commit(event.shiftKey)}
        >{addLabel}</button>
        {#if !editBackground}<span class="note">
          {added > 0 ? $t("workspace.addedPanelStillOpen", { "added": added }) : $t("workspace.holdShiftToAddAndKeepThis")}
        </span>
        {/if}
      </div>
    </div>
  </div>

  {#if !editBackground}<footer>
    <span>{$t("workspace.addAddAndKeepOpenEscClose")}</span>
    <span>{importing ? $t("workspace.pdfImportPaperIgnored") : `${selected?.label ?? ""} · ${sizes.find((size) => size.id === sizeId)?.name ?? ""}`}</span>
  </footer>{/if}
</div>

<style>
  .muted { opacity: var(--disabled); }
  .generators { display: flex; flex-wrap: wrap; gap: 8px; margin-block-end: 8px; }
  .generators :global(.generator-group) { display: grid; gap: 4px; color: var(--muted); font-size: var(--text-sm); }
  .generators :global(button) { min-height: var(--control); padding-inline: 12px; border: 0; background: var(--wash); color: var(--text); border-radius: var(--radius); font: inherit; font-size: var(--text-md); cursor: pointer; }
  .generators :global(button:hover:enabled) { background: var(--wash-strong); }
  .calculation-preview { box-sizing: border-box; padding: 18%; width: 100%; }
  .code-line { display: block; width: 85%; height: 4px; background: #8d9da8; margin-bottom: 7px; }
  .code-line.short { width: 55%; }
  .calculation-preview svg { display: block; width: 100%; margin-top: 16px; }

  /* Centred on the window, not hung off the button that opened it: a panel this size belongs to
     the window. Entry is opacity plus 4px of travel, no scale. */
  .add-page-panel {
    position: fixed;
    z-index: 50;
    top: 50%;
    left: 50%;
    display: flex;
    width: min(1040px, calc(100vw - 32px));
    flex-direction: column;
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    background: var(--panel);
    box-shadow: var(--shadow-modal);
    color: var(--text);
    overflow: hidden;
    transform: translate(-50%, -50%);
    animation: panel-in var(--enter) ease-out;
  }

  @keyframes panel-in {
    from { opacity: 0; transform: translate(-50%, calc(-50% + 4px)); }
  }

  header {
    display: flex;
    flex: none;
    align-items: center;
    padding: 12px 12px 12px 20px;
    border-bottom: 1px solid var(--edge-soft);
    gap: 16px;
  }

  .subject { min-width: 0; flex: 1; }
  .subject strong { display: block; font-size: var(--text-lg); font-weight: 600; line-height: 1.25; }

  .destination {
    display: block;
    margin-top: 4px;
    color: var(--muted);
    font-size: var(--text-sm);
    line-height: 1.4;
  }

  .run {
    display: flex;
    flex: none;
    align-items: center;
    padding: 2px;
    border-radius: var(--radius);
    background: var(--inset);
    gap: 4px;
  }

  .run-label {
    padding: 0 8px;
    color: var(--muted);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .run button {
    width: var(--control-dense);
    height: var(--control-dense);
    border: 0;
    border-radius: var(--radius);
    background: var(--wash);
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    cursor: pointer;
    transition: background var(--fast) ease;
  }

  .run button:hover:enabled { background: var(--wash-strong); }
  .run button:disabled { opacity: var(--disabled); cursor: default; }

  .run output {
    min-width: 24px;
    color: var(--text);
    font-size: var(--text-md);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    text-align: center;
  }

  .close {
    display: grid;
    width: var(--control);
    height: var(--control);
    flex: none;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    place-items: center;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .close:hover { background: var(--wash); color: var(--text); }

  .close svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-width: var(--stroke-dense);
  }

  .body {
    display: flex;
    height: min(520px, calc(100vh - 240px));
    align-items: stretch;
  }

  /* Column A. Fixed width: the shape of a page is four small decisions, and making them scroll is
     what pushed the library out of sight. */
  .settings {
    display: flex;
    width: 264px;
    flex: none;
    flex-direction: column;
    padding: 16px;
    border-right: 1px solid var(--edge-soft);
    gap: 16px;
    overflow-y: auto;
  }

  section { display: flex; flex-direction: column; gap: 8px; }

  .pdf-pages input {
    box-sizing: border-box;
    width: 100%;
    height: var(--control);
    padding: 0 12px;
    border: 1px solid var(--edge);
    border-radius: var(--radius);
    background: var(--inset);
    caret-color: var(--blueprint-light);
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    font-variant-numeric: tabular-nums;
    transition: border-color var(--fast) ease;
  }

  .pdf-pages input::placeholder { color: var(--quiet); }
  .pdf-pages input::selection { background: var(--accent-select); }
  .pdf-pages input:hover { border-color: var(--edge-strong); }
  .pdf-pages input:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  .pdf-pages input[aria-invalid="true"] { border-color: var(--oxide); }
  .pdf-pages .help { color: var(--quiet); font-size: var(--text-sm); }
  .pdf-pages .help.invalid { color: var(--oxide-text); }

  .eyebrow {
    color: var(--muted);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .eyebrow.spread { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }

  .readout {
    font-variant-numeric: tabular-nums;
    font-weight: 400;
  }

  /* Rows wrap rather than squeeze: a label is never cut to fit a chip. */
  .row { display: flex; flex-wrap: wrap; gap: 8px; }
  .pairs { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }

  /* Controls have no border; the selected chip gets the accent ring. */
  .chip {
    display: flex;
    min-height: var(--control);
    flex: 1 1 auto;
    align-items: center;
    justify-content: center;
    padding: 4px 12px;
    border: 0;
    border-radius: var(--radius);
    background: var(--wash);
    color: var(--muted);
    font: inherit;
    font-size: var(--text-md);
    line-height: 1.25;
    text-align: center;
    cursor: pointer;
    gap: 8px;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .chip:hover:enabled { background: var(--wash-strong); color: var(--text); }
  .chip:disabled { opacity: var(--disabled); cursor: default; }

  .chip.on,
  .chip.on:hover:enabled {
    background: var(--accent-fill);
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    color: var(--text);
    font-weight: 600;
  }

  .page-mark {
    width: 9px;
    height: 12px;
    flex: none;
    border: 1px solid currentColor;
    border-radius: 1px;
    transition: width var(--fast) ease, height var(--fast) ease;
  }

  .page-mark.landscape { width: 12px; height: 9px; }

  .swatch {
    width: 16px;
    height: 16px;
    flex: none;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px var(--edge-strong);
  }

  /* Column B. */
  .library {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    border-right: 1px solid var(--edge-soft);
  }

  .band {
    display: flex;
    flex: none;
    flex-direction: column;
    padding: 16px;
    border-bottom: 1px solid var(--edge-soft);
    gap: 12px;
  }

  .filters { display: flex; flex-wrap: wrap; gap: 8px; }

  .pill {
    display: flex;
    min-height: var(--control-dense);
    align-items: center;
    padding: 0 12px;
    border: 0;
    border-radius: var(--radius);
    background: var(--wash);
    color: var(--muted);
    font: inherit;
    font-size: var(--text-md);
    cursor: pointer;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .pill:hover { background: var(--wash-strong); color: var(--text); }

  .pill.on,
  .pill.on:hover {
    background: var(--accent-fill);
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    color: var(--text);
    font-weight: 600;
  }

  /* Side by side, not stacked: two short tiles leave the paper library a row higher. */
  .pins { display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 8px; }

  .import {
    display: flex;
    min-width: 0;
    align-items: center;
    padding: 8px 12px;
    border: 0;
    border-radius: var(--radius);
    background: var(--wash);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    gap: 12px;
    transition: background var(--fast) ease;
  }

  .import:hover:enabled { background: var(--wash-strong); }
  .import:disabled { opacity: var(--disabled); cursor: default; }

  .import.on,
  .import.on:hover:enabled {
    background: var(--accent-fill);
    box-shadow: inset 0 0 0 1px var(--accent-edge);
  }

  .sheet {
    position: relative;
    display: flex;
    width: 26px;
    height: 33px;
    flex: none;
    align-items: flex-end;
    justify-content: center;
    padding-bottom: 4px;
    border-radius: 3px;
    background: var(--paper);
  }

  .sheet :global(svg) { display: block; width: 100%; height: 100%; }

  .badge {
    padding: 1px 3px;
    border-radius: 2px;
    background: var(--charcoal);
    color: var(--text);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .badge.big { padding: 4px 8px; font-size: var(--text-sm); }

  /* Names wrap so a real name reads in full. */
  .words { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
  .words .title { font-size: var(--text-md); font-weight: 600; line-height: 1.25; overflow-wrap: anywhere; }
  .words .sub { color: var(--muted); font-size: var(--text-sm); line-height: 1.4; overflow-wrap: anywhere; }

  .papers {
    min-width: 0;
    flex: 1;
    padding: 16px;
    overflow-y: auto;
    transition: opacity var(--fast) ease;
  }

  .shelf + .shelf { margin-top: 24px; }
  .shelf .eyebrow { margin-bottom: 12px; }

  /* `minmax(0, …)` rather than `1fr`: a track's automatic minimum is its content's min-content
     width, and a preview is an SVG that reports an intrinsic 300px. */
  .cards {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
  }

  .card {
    display: flex;
    flex-direction: column;
    padding: 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    font: inherit;
    text-align: left;
    cursor: pointer;
    gap: 8px;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .card:hover:enabled { background: var(--wash); color: var(--text); }
  .card:disabled { opacity: var(--disabled); cursor: default; }

  .card.on,
  .card.on:hover:enabled {
    background: var(--accent-fill);
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    color: var(--text);
  }

  .thumb {
    display: block;
    width: 100%;
    border-radius: 3px;
    background: var(--paper);
    box-shadow: 0 0 0 1px rgb(0 0 0 / 40%);
    overflow: hidden;
  }

  .thumb :global(svg) { display: block; width: 100%; height: 100%; }

  .name {
    font-size: var(--text-sm);
    line-height: 1.3;
    overflow-wrap: anywhere;
  }

  /* Column C. The page on the desk. */
  .preview {
    --stage-w: 230px;
    --stage-h: 300px;
    display: flex;
    width: 262px;
    flex: none;
    flex-direction: column;
    padding: 16px;
    background: var(--surround);
    gap: 12px;
  }

  .stage {
    display: flex;
    min-height: var(--stage-h);
    flex: 1;
    align-items: center;
    justify-content: center;
  }

  /* Sized from the stage rather than from its contents: `aspect-ratio` on a box with no children
     and no definite side resolves to zero. */
  .sheet-preview {
    width: auto;
    height: min(var(--stage-h), calc(var(--stage-w) / var(--ratio)));
    background: var(--paper);
    box-shadow: var(--shadow-page);
  }

  .sheet-preview :global(svg) { display: block; width: 100%; height: 100%; }
  .sheet-preview.pdf { display: grid; place-items: center; }

  .summary {
    display: flex;
    flex: none;
    flex-direction: column;
    margin: 0;
    gap: 8px;
  }

  .summary div {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .summary dt {
    flex: none;
    color: var(--muted);
    font-size: var(--text-sm);
  }

  .summary dd {
    min-width: 0;
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
    text-align: right;
  }

  .commit { display: flex; flex: none; flex-direction: column; gap: 8px; }

  .add {
    height: var(--control);
    border: 0;
    border-radius: var(--radius);
    background: var(--blueprint);
    color: var(--on-accent);
    font: inherit;
    font-size: var(--text-md);
    font-weight: 600;
    cursor: pointer;
    transition: background var(--fast) ease;
  }

  .add:hover:enabled { background: var(--blueprint-hover); }
  .add:disabled { opacity: var(--disabled); cursor: default; }

  .note { color: var(--quiet); font-size: var(--text-sm); line-height: 1.4; text-align: center; }

  footer {
    display: flex;
    min-height: 36px;
    flex: none;
    align-items: center;
    justify-content: space-between;
    padding: 8px 20px;
    border-top: 1px solid var(--edge-soft);
    background: var(--surround);
    color: var(--quiet);
    font-size: var(--text-sm);
    line-height: 1.4;
    gap: 16px;
  }

  button:focus-visible {
    outline: 2px solid var(--blueprint-light);
    outline-offset: 2px;
  }

  .card:focus-visible,
  .chip:focus-visible,
  .import:focus-visible { outline-offset: -2px; }

  /* A narrower window tightens the two fixed columns and drops the library to three across
     before it gives up the preview. */
  @media (max-width: 1120px) {
    .settings { width: 232px; padding: 12px; }
    .preview { --stage-w: 192px; --stage-h: 250px; width: 224px; padding: 12px; }
    .cards { grid-template-columns: repeat(3, minmax(0, 1fr)); }
    .body { height: min(480px, calc(100vh - 200px)); }
  }

  /* Below this there is no honest way to show a page and a library side by side. */
  @media (max-width: 880px) {
    .preview { display: none; }
    .cards { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }

  @media (pointer: coarse) {
    .chip, .add, .pdf-pages input, .pill { min-height: var(--control-touch, 44px); }
    .run button { width: var(--control-touch, 44px); height: var(--control-touch, 44px); }
  }

  @media (prefers-reduced-motion: reduce) {
    .add-page-panel { animation: none; }

    .chip,
    .pill,
    .card,
    .import,
    .page-mark,
    .papers,
    .add,
    .close,
    .run button { transition: none; }
  }
</style>
