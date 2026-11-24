<script lang="ts">
  import { onMount } from "svelte";
  import { contributionText, type ContributionEntry } from "../ipc/addons";
  import { locale, t } from "../i18n";
  import { dismissable } from "./dismiss";
  import { exportTarget, parsePageList, type ExportChoice, type ExportInclude, type ExportScope } from "./exportTarget";

  /**
   * The export sheet. It borrows the Add page panel's language on purpose, because the two are a
   * pair: what goes into the notebook and what comes out of it. Same header with a live line that
   * says what the button will do, same chips, same dark preview column with the page on a stage,
   * same footer of keys. It is smaller because there are fewer decisions.
   *
   * The preview is a diagram of the kinds of content, not a render of the writer's pages: each
   * kind is drawn while it is switched on, so leaving the highlighter out visibly removes the
   * highlighter. Rendering real pages would mean compiling them every time a chip changes.
   *
   * The Export button takes focus on open without a ring, so repeating the last export is the
   * button plus Enter. The choice is owned by the caller and outlives the sheet.
   */
  let { formats = [], onAddonExport, choice, pageNumber, pageCount, trigger, onChange, onExport, onClose }: {
    formats?: ContributionEntry[];
    onAddonExport?: (entry: ContributionEntry, current: boolean) => void;
    choice: ExportChoice;
    /** The page in view, one-based. */
    pageNumber: number;
    pageCount: number;
    /** The header button: it toggles the sheet itself, so a press on it must not count as outside. */
    trigger?: HTMLElement;
    onChange: (choice: ExportChoice) => void;
    onExport: (choice: ExportChoice) => void;
    onClose: () => void;
  } = $props();

  let selectedFormat = $state("");
  const format = $derived(formats.find(({ addon, command }) => selectedFormat === addon.addonId + "/" + command.id));
  const formatLabel = $derived(format?.command.surface === "export"
    ? contributionText(format.addon, format.command.format.label, $locale) : null);
  let commit = $state<HTMLButtonElement>();
  let pagesInput = $state<HTMLInputElement>();
  onMount(() => commit?.focus({ focusVisible: false } as FocusOptions));

  const scopes: { id: ExportScope; label: () => string }[] = [
    { id: "notebook", label: () => $t("exportMenu.notebook") },
    { id: "page", label: () => $t("exportMenu.thisPage") },
    { id: "pages", label: () => $t("exportMenu.pages") },
  ];

  type Kind = Exclude<keyof ExportInclude, "calculations">;
  // Each chip carries the mark the preview draws for it, so the two read as one key.
  const kinds: { id: Kind; label: () => string; icon: string }[] = [
    { id: "handwriting", label: () => $t("exportMenu.handwriting"), icon: "M3 15c2-5 4-7 5-4s2 5 4 1 3-6 5-3 2 4 4 2" },
    { id: "highlighter", label: () => $t("exportMenu.highlighter"), icon: "M3 9h18v6H3zM6 12h9" },
    { id: "typst", label: () => $t("exportMenu.typst"), icon: "M6 5h12M12 5v14M9 19h6" },
    { id: "pageText", label: () => $t("exportMenu.pageText"), icon: "M4 6h16M4 10h16M4 14h16M4 18h10" },
    { id: "images", label: () => $t("exportMenu.images"), icon: "M4 5h16v14H4zM4 16l5-5 4 4 2-2 5 5M15.5 9.5h.01" },
    { id: "shapes", label: () => $t("exportMenu.shapes"), icon: "M8.5 4a5 5 0 1 0 0 10 5 5 0 0 0 0-10M13 13h7v7h-7z" },
    { id: "background", label: () => $t("exportMenu.paper"), icon: "M6 3h12v18H6zM9 8h6M9 12h6M9 16h6" },
  ];

  const jupyterChoices = [
    { id: "out", label: () => $t("exportMenu.leaveOut") },
    { id: "full", label: () => $t("exportMenu.fullLength") },
    { id: "a4", label: () => $t("exportMenu.a4Sheets") },
  ] as const;
  const jupyter = $derived(!choice.include.calculations ? "out" : choice.splitCalculationsA4 ? "a4" : "full");

  const listed = $derived(choice.scope === "pages" ? parsePageList(choice.pages, pageCount) : null);
  // The real ids do not matter for counting and naming; their positions do.
  const target = $derived(exportTarget(choice, Array.from({ length: pageCount }, (_, index) => String(index + 1)), String(pageNumber)));
  const outputName = $derived(format?.command.surface === "export"
    ? `notebook.${format.command.format.extension}` : target?.outputName);
  const count = $derived(target?.pageCount ?? 0);
  const pagesReadout = $derived(
    choice.scope === "notebook"
      ? $t("exportMenu.allPages", { count: pageCount })
      : choice.scope === "page"
        ? $t("exportMenu.pageN", { number: pageNumber })
        : listed ? choice.pages.trim() : "—",
  );
  const headline = $derived(target ? $t("exportMenu.headline", { count, file: outputName ?? target.outputName }) : $t("exportMenu.pagesInvalid", { pageCount }));

  function pick(scope: ExportScope) {
    // A fresh list starts at the page in view, which is the likelier start than page 1.
    const pages = scope === "pages" && !choice.pages.trim() ? `${pageNumber}-${pageCount}` : choice.pages;
    onChange({ ...choice, scope, pages });
    if (scope === "pages") queueMicrotask(() => pagesInput?.select());
  }

  function pickJupyter(id: (typeof jupyterChoices)[number]["id"]) {
    onChange({ ...choice, include: { ...choice.include, calculations: id !== "out" }, splitCalculationsA4: id === "a4" ? true : id === "full" ? false : choice.splitCalculationsA4 });
  }

  function toggle(kind: Kind) {
    onChange({ ...choice, include: { ...choice.include, [kind]: !choice.include[kind] } });
  }

  /** Arrow keys within a radio row, the same as the Add page chips. */
  function walk(event: KeyboardEvent) {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const radios = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("[role=radio]:not(:disabled)")];
    const next = radios[(radios.indexOf(document.activeElement as HTMLButtonElement) + step + radios.length) % radios.length];
    next?.click();
    next?.focus();
  }

  function run() {
    if (!count) return;
    if (format) onAddonExport?.(format, choice.scope === "page");
    else onExport(choice);
    onClose();
  }

  function keydown(event: KeyboardEvent) {
    const onButton = (event.target as HTMLElement).closest("button");
    if (event.key === "Enter" && !onButton && !(event.target instanceof HTMLSelectElement)) {
      event.preventDefault();
      run();
    }
  }
</script>

<div
  class="export-panel"
  data-ui="solid"
  role="dialog"
  tabindex="-1"
  aria-labelledby="export-title"
  aria-describedby="export-headline"
  use:dismissable={{ onClose, trigger }}
  onkeydown={keydown}
>
  <header>
    <div class="subject">
      <strong id="export-title">{formatLabel ?? $t("workspace.exportPDF")}</strong>
      <span id="export-headline" class="headline" class:invalid={!target}>{headline}</span>
    </div>
    <button type="button" class="close" aria-label={$t("library.close")} title={$t("library.close")} onclick={onClose}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
    </button>
  </header>

  <div class="body">
    <div class="settings">
      {#if formats.length}
        <label class="format">
          {$t("addons.contributions.format")}
          <select aria-label={$t("addons.contributions.format")} value={selectedFormat} onchange={(event) => {
            selectedFormat = event.currentTarget.value;
            if (selectedFormat && choice.scope === "pages") pick("notebook");
          }}>
            <option value="">PDF</option>
            {#each formats as { addon, command }}
              {#if command.surface === "export"}
                <option value={addon.addonId + "/" + command.id}>{contributionText(addon, command.format.label, $locale)} (.{command.format.extension})</option>
              {/if}
            {/each}
          </select>
        </label>
      {/if}
      {#if pageCount > 1}
        <section>
          <div class="eyebrow" id="export-pages">{$t("exportMenu.pages")}</div>
          <div class="row" role="radiogroup" aria-labelledby="export-pages" tabindex="-1" onkeydown={walk}>
            {#each scopes.filter(scope => !format || scope.id !== "pages") as scope (scope.id)}
              <button
                type="button"
                role="radio"
                class="chip"
                class:on={choice.scope === scope.id}
                aria-checked={choice.scope === scope.id}
                tabindex={choice.scope === scope.id ? 0 : -1}
                onclick={() => pick(scope.id)}
              >{scope.label()}</button>
            {/each}
          </div>
          {#if choice.scope === "pages"}
            <div class="pages">
              <input
                bind:this={pagesInput}
                type="text"
                inputmode="numeric"
                spellcheck="false"
                autocomplete="off"
                value={choice.pages}
                placeholder={$t("exportMenu.pagesPlaceholder")}
                aria-label={$t("exportMenu.pagesLabel")}
                aria-invalid={listed === null}
                oninput={(event) => onChange({ ...choice, pages: event.currentTarget.value })}
              />
              <span class="of">{$t("workspace.of", { pageCount })}</span>
            </div>
          {/if}
        </section>
      {/if}

      {#if !format}
      <section>
        <div class="eyebrow">{$t("exportMenu.include")}</div>
        <div class="pairs" role="group" aria-label={$t("exportMenu.include")}>
          {#each kinds as kind (kind.id)}
            <button
              type="button"
              class="chip toggle"
              class:on={choice.include[kind.id]}
              class:wide={kind.id === "background"}
              aria-pressed={choice.include[kind.id]}
              onclick={() => toggle(kind.id)}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d={kind.icon} /></svg>
              <span>{kind.label()}</span>
            </button>
          {/each}
        </div>
      </section>

      <section>
        <div class="eyebrow" id="export-jupyter">{$t("exportMenu.calculations")}</div>
        <div class="row" role="radiogroup" aria-labelledby="export-jupyter" tabindex="-1" onkeydown={walk}>
          {#each jupyterChoices as option (option.id)}
            <button
              type="button"
              role="radio"
              class="chip"
              class:on={jupyter === option.id}
              aria-checked={jupyter === option.id}
              tabindex={jupyter === option.id ? 0 : -1}
              title={option.id === "a4" ? $t("exportMenu.splitA4Detail") : undefined}
              onclick={() => pickJupyter(option.id)}
            >{option.label()}</button>
          {/each}
        </div>
      </section>
      {/if}
    </div>

    <div class="preview">
      <div class="eyebrow">{$t("exportMenu.contents")}</div>
      {#if !format}
      <div class="stage">
        <div class="stack" class:many={count > 1}>
          <!-- Notebook content colours, drawn on paper: ink, highlighter, a photo. Not UI tokens. -->
          <svg class="sheet" viewBox="0 0 210 297" aria-hidden="true">
            <rect width="210" height="297" fill="#fdfdfb" />
            {#if choice.include.background}
              <g stroke="#cfdbe6" stroke-width="1">
                {#each Array.from({ length: 21 }, (_, index) => 34 + index * 12.5) as y (y)}<line x1="0" y1={y} x2="210" y2={y} />{/each}
              </g>
              <line x1="28" y1="0" x2="28" y2="297" stroke="#efc7c4" stroke-width="1" />
            {/if}
            {#if choice.include.highlighter}
              <rect x="40" y="52" width="92" height="11" rx="2" fill="#f7e35a" fill-opacity="0.55" />
            {/if}
            {#if choice.include.pageText}
              <g fill="#5d6670">
                <rect x="40" y="31" width="130" height="5" rx="1.5" />
                <rect x="40" y="43.5" width="142" height="5" rx="1.5" />
                <rect x="40" y="56" width="86" height="5" rx="1.5" />
              </g>
            {/if}
            {#if choice.include.typst}
              <g fill="#20262c">
                <rect x="52" y="86" width="22" height="5" rx="1.5" />
                <rect x="80" y="80" width="40" height="4.5" rx="1.5" />
                <rect x="78" y="88" width="44" height="1.6" />
                <rect x="86" y="94" width="28" height="4.5" rx="1.5" />
                <rect x="128" y="86" width="30" height="5" rx="1.5" />
              </g>
            {/if}
            {#if choice.include.handwriting}
              <path d="M42 140c8-16 14-18 18-6s9 14 15 1 12-15 17-4 8 12 14 3 10-10 15-2M44 166c10-6 22-9 40-6s30 1 44-5"
                fill="none" stroke="#1e3a6b" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" />
            {/if}
            {#if choice.include.images}
              <g>
                <rect x="40" y="188" width="70" height="52" rx="2" fill="#dbe6ee" />
                <path d="M40 232l20-20 15 14 9-8 26 22v0H40z" fill="#9db4c6" />
                <circle cx="93" cy="202" r="5" fill="#f3d27a" />
              </g>
            {/if}
            {#if choice.include.shapes}
              <g fill="none" stroke="#2b6cb0" stroke-width="2">
                <circle cx="146" cy="204" r="16" />
                <path d="M128 236h44M164 230l8 6-8 6" stroke-linecap="round" stroke-linejoin="round" />
              </g>
            {/if}
          </svg>
        </div>
      </div>

      {/if}
      <dl class="summary">
        <div><dt>{$t("exportMenu.pages")}</dt><dd>{pagesReadout}</dd></div>
        {#if !format}
          <div><dt>{$t("exportMenu.calculations")}</dt><dd>{jupyterChoices.find((option) => option.id === jupyter)?.label()}</dd></div>
        {/if}
        <div><dt>{$t("exportMenu.file")}</dt><dd title={outputName}>{outputName ?? "—"}</dd></div>
      </dl>

      <div class="commit">
        <button type="button" class="export" bind:this={commit} disabled={!count} onclick={run}>{$t("exportMenu.export", { count })}</button>
        <span class="note">{$t(format ? "addons.contributions.destination" : "exportMenu.destination")}</span>
      </div>
    </div>
  </div>

  <footer>
    <span>{$t("exportMenu.keys")}</span>
    <span>{$t("exportMenu.unchanged")}</span>
  </footer>
</div>

<style>
  /* Field with its label above: 12px/600 muted, 14px text inside. */
  .format { display: grid; gap: 8px; color: var(--muted); font-size: var(--text-sm); font-weight: 600; }

  .format select {
    min-height: var(--control);
    padding: 0 12px;
    border: 1px solid var(--edge);
    border-radius: var(--radius);
    background: var(--inset);
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    font-weight: 400;
  }

  .format select:hover { border-color: var(--edge-strong); }

  /* Centred on the window like the Add page panel, which it is paired with. Entry is opacity plus
     4px of travel, no scale. */
  .export-panel {
    position: fixed;
    z-index: 50;
    top: 50%;
    left: 50%;
    display: flex;
    width: min(660px, calc(100vw - 32px));
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
  .subject strong { display: block; color: var(--text); font-size: var(--text-lg); font-weight: 600; line-height: 1.25; }

  .headline {
    display: block;
    margin-top: 4px;
    color: var(--muted);
    font-size: var(--text-sm);
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  .headline.invalid { color: var(--oxide-text); }

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

  .body { display: flex; align-items: stretch; }

  .settings {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    padding: 16px;
    border-right: 1px solid var(--edge-soft);
    gap: 16px;
  }

  section { display: flex; flex-direction: column; gap: 8px; }

  .eyebrow {
    color: var(--muted);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .row { display: flex; flex-wrap: wrap; gap: 8px; }
  .pairs { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }

  /* Controls have no border; the selected chip gets the accent ring. */
  .chip {
    display: flex;
    min-width: 0;
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

  .chip.on,
  .chip.on:hover:enabled {
    background: var(--accent-fill);
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    color: var(--text);
    font-weight: 600;
  }

  .chip.toggle { justify-content: flex-start; text-align: left; }
  .chip.wide { grid-column: 1 / -1; }

  .chip svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: var(--stroke-dense);
  }

  .pages { display: flex; align-items: center; gap: 8px; }

  .pages input {
    box-sizing: border-box;
    min-width: 0;
    height: var(--control);
    flex: 1;
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

  .pages input::placeholder { color: var(--quiet); }
  .pages input::selection { background: var(--accent-select); }
  .pages input:hover { border-color: var(--edge-strong); }
  .pages input:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  .pages input[aria-invalid="true"] { border-color: var(--oxide); }
  .of { flex: none; color: var(--quiet); font-size: var(--text-sm); font-variant-numeric: tabular-nums; }

  .preview {
    display: flex;
    width: 244px;
    flex: none;
    flex-direction: column;
    padding: 16px;
    background: var(--surround);
    gap: 12px;
  }

  .stage {
    display: flex;
    min-height: 196px;
    flex: 1;
    align-items: center;
    justify-content: center;
  }

  /* More than one page shows as a short stack: the sheets behind say "several" without a number
     competing with the one in the summary. */
  .stack { position: relative; width: 132px; aspect-ratio: 210 / 297; }

  .stack.many::before,
  .stack.many::after {
    position: absolute;
    inset: 0;
    background: var(--paper);
    box-shadow: var(--shadow-page);
    content: "";
  }

  .stack.many::before { transform: translate(8px, -8px); }
  .stack.many::after { transform: translate(4px, -4px); }

  .sheet {
    position: relative;
    z-index: 1;
    display: block;
    width: 100%;
    height: 100%;
    box-shadow: var(--shadow-page);
  }

  .summary { display: flex; flex: none; flex-direction: column; margin: 0; gap: 8px; }
  .summary div { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
  .summary dt { flex: none; color: var(--muted); font-size: var(--text-sm); }

  .summary dd {
    min-width: 0;
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
    text-align: right;
  }

  .commit { display: flex; flex: none; flex-direction: column; gap: 8px; }

  .export {
    height: var(--control);
    border: 0;
    border-radius: var(--radius);
    background: var(--blueprint);
    color: var(--on-accent);
    font: inherit;
    font-size: var(--text-md);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition: background var(--fast) ease;
  }

  .export:hover:enabled { background: var(--blueprint-hover); }
  .export:disabled { opacity: var(--disabled); cursor: default; }
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

  .chip:focus-visible { outline-offset: -2px; }

  /* Too narrow for the page beside the settings: the header line still says what will happen. */
  @media (max-width: 640px) {
    .preview .stage, .preview > .eyebrow { display: none; }
    .preview { width: 200px; }
  }

  @media (pointer: coarse) {
    .chip, .export, .pages input { min-height: var(--control-touch, 44px); }
  }

  @media (prefers-reduced-motion: reduce) {
    .export-panel { animation: none; }
    .chip, .export, .pages input, .close { transition: none; }
  }
</style>
