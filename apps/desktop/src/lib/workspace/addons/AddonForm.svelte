<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { locale, t } from "../../i18n";
  import { cancelAddonCommand, contributionText, dismissAddonForm, previewAddonForm, type AddonValues, type ContributionEntry, type FormPreview } from "../../ipc/addons";
  import type { PageGeometry } from "../../model";
  import { templatePreviewSvg } from "../../page/template";
  import { shapeCaps, supportsShapeCaps } from "../../shape/geometry";
  import ShapeMarks from "../../notebook/ShapeMarks.svelte";
  import { dismissable } from "../dismiss";
  import AddonField from "./AddonField.svelte";
  import { accepts, formDefaults } from "./formModel";

  let { entries, root, pageId, geometry = null, flush, commit, onClose }: {
    entries: ContributionEntry[];
    root: string;
    pageId: string;
    geometry?: PageGeometry | null;
    flush: () => Promise<boolean>;
    commit: (preview: FormPreview, direct: boolean) => Promise<boolean>;
    onClose: () => void;
  } = $props();

  const generator = $derived(geometry !== null);
  let selected = $state(0);
  const entry = $derived(entries[selected]);
  const skipReview = $derived(!generator && entry?.addon.directApply === true);
  const fields = $derived(entry && (entry.command.surface === "insert-form" || entry.command.surface === "page-generator") ? entry.command.form : []);
  let values = $state<AddonValues>({});
  let preview = $state<FormPreview>();
  let pending = $state(false);
  let attempted = $state(false);
  let error = $state<"invalid" | "failed">();
  let panel: HTMLElement;
  let requestId: string | undefined;
  let alive = true;
  let returnFocus: HTMLElement | null;
  const title = $derived(generator ? $t("workspace.addPage") : $t("addons.contributions.insertTitle"));

  onMount(() => {
    returnFocus = document.activeElement as HTMLElement | null;
    select(0);
    panel.querySelector<HTMLElement>("select, .preview-button")?.focus();
  });
  onDestroy(() => {
    alive = false;
    if (requestId) void cancelAddonCommand(requestId).catch(() => {});
    clearPreview();
    returnFocus?.focus();
  });

  function clearPreview() {
    if (preview) void dismissAddonForm(preview.id).catch(() => {});
    preview = undefined;
  }

  function select(index: number) {
    clearPreview();
    selected = index;
    values = formDefaults(fields);
    attempted = false;
    error = undefined;
  }

  async function generate() {
    if (pending || !entry) return;
    attempted = true;
    if (!fields.every(field => accepts(field, values[field.id]))) {
      error = "invalid";
      return;
    }
    pending = true;
    error = undefined;
    clearPreview();
    try {
      if (!await flush() || !alive) throw new Error("flush_failed");
      requestId = crypto.randomUUID();
      const result = await previewAddonForm({
        root, pageId, addonId: entry.addon.addonId, command: entry.command.id,
        requestId, form: values, reviewFirst: !skipReview, geometry, anchor: [0, 0],
      });
      if (!alive) {
        void dismissAddonForm(result.id).catch(() => {});
        return;
      }
      preview = result;
      if (result.direct && !generator && await commit(result, true)) onClose();
      else {
        await tick();
        panel.querySelector<HTMLElement>(".insert")?.focus();
      }
    } catch {
      if (alive) error = "failed";
    } finally {
      requestId = undefined;
      pending = false;
    }
  }

  async function insert() {
    if (!preview || pending) return;
    pending = true;
    try {
      if (await commit(preview, false)) onClose();
    } catch {
      error = "failed";
    } finally {
      pending = false;
    }
  }

  function trap(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const controls = [...panel.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled), select:not(:disabled)")];
    const first = controls[0], last = controls.at(-1);
    if (event.shiftKey && document.activeElement === first && last) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last && first) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<div role="dialog" aria-modal="true" aria-label={title} class="sheet" tabindex="-1"
  bind:this={panel} use:dismissable={{ onClose, swallow: true }} onkeydown={trap}>
  <header>
    <h2>{title}</h2>
    <button type="button" class="close" aria-label={$t("library.close")} onclick={onClose}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
    </button>
  </header>
  <div class="body">
    <form onsubmit={(event) => { event.preventDefault(); void generate(); }} novalidate>
      {#if entries.length > 1}
        <label>
          {$t("addons.contributions.choose")}
          <select value={selected} disabled={pending} onchange={event => select(Number(event.currentTarget.value))}>
            {#each entries as { addon, command }, index}
              <option value={index}>{contributionText(addon, "name", $locale)} · {contributionText(addon, command.label, $locale)}</option>
            {/each}
          </select>
        </label>
      {:else if entry}
        <strong class="subject">{contributionText(entry.addon, entry.command.label, $locale)}</strong>
        <p>{contributionText(entry.addon, "name", $locale)}</p>
      {/if}
      {#if entry}
        {#each fields as field (field.id)}
          <AddonField addon={entry.addon} {field} value={values[field.id]} disabled={pending}
            invalid={attempted && !accepts(field, values[field.id])}
            onChange={(value) => { values[field.id] = value; clearPreview(); error = undefined; }} />
        {/each}
      {/if}
      <button type="submit" class="preview-button" disabled={pending}>{$t(skipReview ? "addons.contributions.insert" : "addons.contributions.preview")}</button>
      {#if error}<p role="alert">{$t(`addons.contributions.${error}`)}</p>{/if}
    </form>
    <div class="preview" aria-label={$t("addons.contributions.preview")} aria-busy={pending}>
      {#if preview}
        {#each preview.pages as page}
          <div class="paper" style:aspect-ratio={preview.geometry.widthPt / preview.geometry.heightPt} style:background={page.background.kind === "plain" ? page.background.color : undefined}>
            {#if page.background.kind === "template"}
              <div class="template">{@html templatePreviewSvg(page.background.template, preview.geometry)}</div>
            {/if}
            <svg viewBox={`0 0 ${preview.geometry.widthPt} ${preview.geometry.heightPt}`} aria-hidden="true">
              {#each page.items as item (item.object.id)}
                {#if item.object.type === "shape"}
                  {@const object = item.object}
                  <g transform={`translate(${object.x} ${object.y})`}>
                    <ShapeMarks marks={shapeCaps(object.geometry, object.style)} style={object.style}
                      closed={!supportsShapeCaps(object.geometry)} />
                  </g>
                {:else if item.svg}
                  <image href={`data:image/svg+xml,${encodeURIComponent(item.svg)}`} x={item.object.x} y={item.object.y} width={item.widthPt} height={item.heightPt} />
                {/if}
              {/each}
            </svg>
          </div>
        {/each}
      {:else}
        <p>{$t("addons.contributions.emptyPreview")}</p>
      {/if}
    </div>
  </div>
  <footer>
    <button class="insert" type="button" disabled={!preview || pending} onclick={() => void insert()}>
      {$t(generator ? "workspace.addPage" : "addons.contributions.insert")}
    </button>
  </footer>
</div>

<style>
  .sheet { position: fixed; z-index: 90; top: 50%; left: 50%; transform: translate(-50%, -50%); display: flex; flex-direction: column; width: min(700px, calc(100vw - 48px)); max-height: calc(100vh - 80px); background: var(--panel); color: var(--text); border: 1px solid var(--edge); border-radius: var(--radius-lg); box-shadow: var(--shadow-modal); overflow: hidden; font: var(--text-md) var(--font-ui); animation: sheet-in var(--enter) ease-out; }
  .sheet:focus { outline: none; }
  @keyframes sheet-in { from { opacity: 0; margin-top: 4px; } }
  header, footer { display: flex; align-items: center; justify-content: space-between; padding: 16px 24px; }
  header { padding-inline: 24px 16px; border-bottom: 1px solid var(--edge-soft); }
  footer { justify-content: flex-end; border-top: 1px solid var(--edge-soft); }
  h2 { margin: 0; font-family: var(--font-display); font-size: var(--text-xl); font-weight: 600; line-height: 1.25; }
  button { display: inline-flex; min-height: var(--control); align-items: center; justify-content: center; padding-inline: 16px; border: 0; border-radius: var(--radius); background: transparent; color: var(--text); font: inherit; font-size: var(--text-md); cursor: pointer; transition: background var(--fast) ease, color var(--fast) ease; }
  button:focus-visible, select:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  button:disabled { opacity: var(--disabled); cursor: default; }
  .close { width: var(--control); padding: 0; color: var(--muted); }
  .close:hover { background: var(--wash); color: var(--text); }
  .close svg { width: var(--icon-dense); height: var(--icon-dense); fill: none; stroke: currentColor; stroke-linecap: round; stroke-width: var(--stroke-dense); }
  .body { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); min-height: 0; overflow: auto; }
  form, .preview { min-width: 0; padding: 24px; }
  form > p, .preview > p { margin-block: 4px 0; color: var(--muted); font-size: var(--text-sm); line-height: 1.4; }
  form > p[role="alert"] { color: var(--oxide-text); font-size: var(--text-md); }
  .subject { display: block; font-size: var(--text-md); font-weight: 600; }
  form > label { display: grid; gap: 4px; color: var(--muted); font-size: var(--text-sm); font-weight: 600; }
  select { box-sizing: border-box; width: 100%; min-height: var(--control); padding: 0 12px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); color: var(--text); color-scheme: dark; font: inherit; font-size: var(--text-md); font-weight: 400; transition: border-color var(--fast) ease; }
  select:hover:not(:disabled) { border-color: var(--edge-strong); }
  .preview { display: grid; align-content: start; justify-items: center; gap: 12px; background: var(--charcoal); border-inline-start: 1px solid var(--edge-soft); }
  .paper { width: min(100%, 200px); position: relative; background: var(--paper); overflow: hidden; }
  .paper > svg, .template { position: absolute; inset: 0; width: 100%; height: 100%; }
  .template :global(svg) { width: 100%; height: 100%; }
  .preview-button { margin-top: 12px; background: var(--wash); }
  .preview-button:hover:not(:disabled) { background: var(--wash-strong); }
  .insert { background: var(--blueprint); color: var(--on-accent); font-weight: 600; }
  .insert:hover:not(:disabled) { background: var(--blueprint-hover); }
  @media (max-width: 540px) { .body { grid-template-columns: minmax(0, 1fr); } .preview { border-inline-start: 0; border-top: 1px solid var(--edge-soft); } }
  @media (pointer: coarse) { button, select { min-height: var(--control-touch); } .close { width: var(--control-touch); } }
  @media (prefers-reduced-motion: reduce) { .sheet { animation: none; } button, select { transition: none; } }
</style>
