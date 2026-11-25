<script lang="ts" module>
  // The writer's language choice holds for the session, so a German notebook stays German.
  let rememberedRecognizer = "";
</script>

<script lang="ts">
  import { onMount, tick } from "svelte";
  import { t, notice, renderMessage, type UiText } from "../i18n";
  import { errorDetails } from "../ipc/errors";
  import {
    prepareMathRecognition,
    recognizeInkMath,
    recognizeInkText,
    removeMathRecognizer,
    type InkPolylines,
    type MathContext,
  } from "../ipc/recognition";
  import AddonApproval from "../workspace/AddonApproval.svelte";
  import AddonPermissions from "../workspace/AddonPermissions.svelte";
  import ConfirmDialog from "../workspace/ConfirmDialog.svelte";
  import { ADDON_AVAILABILITY_CHANGED, mathRecognitionAddon, type RecognitionAddon } from "../ipc/addons";
  import { cancelImporter, importerStatus, installImporter, megabytes, type ImporterStatus } from "../ipc/importer";
  import type { Bounds } from "../ink/selection";
  import { editDistance, transcriptionSource, type TranscriptionKind } from "./transcription";

  /**
   * Review before insertion (ADR 0040): the recognizer proposes, the writer corrects beside the
   * original ink, and only a confirmed result becomes a Typst block. The ink is never changed.
   */
  type Props = {
    polylines: InkPolylines;
    bounds: Bounds;
    context: MathContext;
    recognizers: string[];
    math: ImporterStatus | null;
    mathAddon: RecognitionAddon | null;
    onInsert: (source: string, label: UiText) => void;
    onClose: () => void;
  };
  let { polylines, bounds, context, recognizers, math, mathAddon, onInsert, onClose }: Props = $props();

  let addon = $state<ImporterStatus | null>(null);
  let addonBusy = $state(false);
  let confirmingRemoval = $state(false);
  let kind = $state<TranscriptionKind | null>(null);
  let busy = $state(false);
  /** A status line; `problem` ones are failures the writer has to act on. */
  let message = $state<{ text: UiText; problem: boolean } | null>(null);
  let value = $state("");
  let rawLatex = $state("");
  let recognizer = $state(rememberedRecognizer);
  let field = $state<HTMLTextAreaElement>();
  let firstChoice = $state<HTMLButtonElement>();
  let warmed = false;
  let proposal = "";
  let shownAt = 0;
  let request = 0;
  let disposed = false;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;

  const pad = 6;
  const viewBox = $derived(
    `${bounds.left - pad} ${bounds.top - pad} ${Math.max(bounds.right - bounds.left, 1) + pad * 2} ${Math.max(bounds.bottom - bounds.top, 1) + pad * 2}`,
  );
  const transferring = $derived(addon?.activity.kind === "downloading" || addon?.activity.kind === "installing");
  /** Math was chosen but its add-on is missing: the panel offers it instead of a result. */
  const offering = $derived(kind === "math" && !addon?.installed);

  function fail(error: unknown) {
    message = { text: notice("transcription.failed", { details: errorDetails(error) }), problem: true };
  }

  let loadedAddon = $state<RecognitionAddon | null>(null);
  /** The workspace loads the catalog entry up front, so Math shows immediately; refresh keeps it current. */
  const recognizerAddon = $derived(loadedAddon ?? mathAddon);

  /** Re-read the add-on, polling only while a download or install is running. */
  async function refresh() {
    clearTimeout(pollTimer);
    try {
      const entry = await mathRecognitionAddon();
      if (disposed || !entry) return;
      loadedAddon = entry;
      const found = await importerStatus(entry.id);
      if (disposed) return;
      addon = found;
      if (found.disabled && kind === "math") {
        request++;
        kind = null;
        message = null;
        busy = false;
        warmed = false;
      }
      if (found.installed && !warmed) {
        // Load the model now, while the writer is still deciding, not when they press Math.
        warmed = true;
        void prepareMathRecognition().catch((error) => { if (!disposed) fail(error); });
      }
      if (["downloading", "installing", "awaitingApproval"].includes(found.activity.kind)) {
        pollTimer = setTimeout(() => void refresh(), 400);
      }
    } catch (error) {
      if (!disposed) fail(error);
    }
  }

  onMount(() => {
    addon = math;
    void refresh();
    firstChoice?.focus();
    window.addEventListener(ADDON_AVAILABILITY_CHANGED, refresh);
    return () => {
      disposed = true;
      clearTimeout(pollTimer);
      request++;
      window.removeEventListener(ADDON_AVAILABILITY_CHANGED, refresh);
    };
  });

  async function download() {
    if (!recognizerAddon) return;
    addonBusy = true;
    message = null;
    try {
      await installImporter(recognizerAddon.id);
    } catch (error) {
      fail(error);
    } finally {
      addonBusy = false;
      void refresh();
    }
  }

  async function cancelDownload() {
    if (!recognizerAddon) return;
    try { await cancelImporter(recognizerAddon.id); } catch (error) { fail(error); }
    void refresh();
  }

  async function removeAddon() {
    addonBusy = true;
    try {
      await removeMathRecognizer();
      warmed = false;
      if (kind === "math") kind = null;
    } catch (error) {
      fail(error);
    } finally {
      addonBusy = false;
      void refresh();
    }
  }

  async function recognize(next: TranscriptionKind) {
    const id = ++request;
    kind = next;
    rawLatex = "";
    message = null;
    if (next === "math" && !addon?.installed) return;
    busy = true;
    message = { text: notice("transcription.recognizing"), problem: false };
    try {
      if (next === "text") {
        rememberedRecognizer = recognizer;
        const result = await recognizeInkText(polylines, recognizer || null);
        if (id === request) await show(result.text, null);
      } else {
        const result = await recognizeInkMath(polylines, context);
        if (id !== request) return;
        if (result.typst === null) {
          rawLatex = result.latex;
          await show("", {
            text: result.unsupported
              ? notice("transcription.unsupported", { detail: result.unsupported })
              : notice("transcription.unsupportedLatex"),
            problem: true,
          });
        } else {
          await show(result.typst, null);
        }
      }
    } catch (error) {
      if (id !== request) return;
      kind = null;
      fail(error);
    } finally {
      if (id === request) busy = false;
    }
  }

  async function show(text: string, note: typeof message) {
    proposal = text;
    value = text;
    message = note;
    busy = false;
    shownAt = performance.now();
    await tick();
    field?.focus();
  }

  function insert() {
    if (!kind || busy || offering || !value.trim()) return;
    const corrections = editDistance(proposal, value);
    const seconds = Math.max(0, Math.round((performance.now() - shownAt) / 1000));
    onInsert(transcriptionSource(kind, value), notice("transcription.insertedWithCorrections", { count: corrections, seconds }));
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    } else if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      insert();
    }
  }
</script>

<div class="transcription" role="dialog" aria-modal="false" aria-labelledby="transcription-title" tabindex="-1" onkeydown={keydown}>
  <header>
    <h2 id="transcription-title">{$t("transcription.title")}</h2>
    <button type="button" class="chrome" title={$t("transcription.close")} aria-label={$t("transcription.close")} onclick={onClose}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"></path></svg>
    </button>
  </header>

  <div class="body">
    <svg class="original" {viewBox} preserveAspectRatio="xMidYMid meet" role="img" aria-label={$t("transcription.original")}>
      {#each polylines as line, index (index)}
        <polyline points={line.points.map(([x, y]) => `${x},${y}`).join(" ")} stroke-width={Math.max(line.widthPt, 0.6)}></polyline>
      {/each}
    </svg>

    <section>
      <span class="section-label" id="transcription-kind">{$t("transcription.recognizeAs")}</span>
      <div class="kinds" role="group" aria-labelledby="transcription-kind">
        <button bind:this={firstChoice} type="button" class:current={kind === "text"} aria-pressed={kind === "text"}
          disabled={busy || recognizers.length === 0} onclick={() => void recognize("text")}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6.5h14M12 6.5V18M9 18h6"></path></svg>
          <span>{$t("transcription.kindText")}</span>
        </button>
        {#if recognizerAddon && !(addon ?? math)?.disabled}
          <button type="button" class:current={kind === "math"} aria-pressed={kind === "math"}
            disabled={busy} onclick={() => void recognize("math")}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14M9.5 4.5l3 3 3-3M9.5 16.5l3 3 3-3"></path></svg>
            <span>{$t(`${recognizerAddon.i18nPrefix}.label`)}</span>
          </button>
        {/if}
      </div>
      {#if recognizers.length > 1 && kind !== "math"}
        <label class="language">
          <span class="section-label">{$t("transcription.language")}</span>
          <span class="select-shell">
            <select bind:value={recognizer} disabled={busy}>
              <option value="">{$t("transcription.systemDefault")}</option>
              {#each recognizers as name (name)}
                <option value={name}>{name}</option>
              {/each}
            </select>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m7 10 5 5 5-5"></path></svg>
          </span>
        </label>
      {/if}
    </section>

    {#if message}
      <p class="message" class:problem={message.problem} role={message.problem ? "alert" : "status"}>{renderMessage(message.text, $t)}</p>
    {/if}

    {#if offering && recognizerAddon && !(addon ?? math)?.disabled}
      <section class="offer" aria-live="polite">
        {#if addon?.activity.kind === "downloading"}
          <p class="lead">{$t("transcription.downloadingAddon")}</p>
          <div class="progress" role="progressbar" aria-label={$t("transcription.download")}
            aria-valuemin="0" aria-valuemax={addon.activity.total} aria-valuenow={addon.activity.received}>
            <span style:transform={`scaleX(${addon.activity.received / Math.max(addon.activity.total, 1)})`}></span>
          </div>
          <div class="row">
            <span class="figure">{$t("transcription.downloadProgress", { received: megabytes(addon.activity.received), total: megabytes(addon.activity.total) })}</span>
            <button type="button" class="control" onclick={() => void cancelDownload()}>{$t("transcription.cancel")}</button>
          </div>
        {:else if addon?.activity.kind === "awaitingApproval"}
          {#key addon.activity.approval.token}
            <AddonApproval id={recognizerAddon.id} approval={addon.activity.approval} {refresh} />
          {/key}
        {:else if addon?.activity.kind === "installing"}
          <p class="lead">{$t("transcription.installingAddon")}</p>
          <div class="progress indeterminate" role="progressbar" aria-label={$t("transcription.installation")}><span></span></div>
        {:else}
          <p class="lead">{$t(`${recognizerAddon.i18nPrefix}.description`)}</p>
          {#if addon?.release}
            <AddonPermissions permissions={addon.catalogPermissions} />
          {/if}
          <dl class="facts">
            <dt>{$t("transcription.factSource")}</dt>
            <dd>{addon?.release ? $t("transcription.factSourceRelease", { size: megabytes(addon.release.sizeBytes) }) : $t("transcription.factSourceNone")}</dd>
            <dt>{$t("transcription.factSent")}</dt>
            <dd>{$t("transcription.factSentValue")}</dd>
            <dt>{$t("transcription.factAfter")}</dt>
            <dd>{$t("transcription.factAfterValue")}</dd>
          </dl>
          {#if addon?.activity.kind === "installFailed"}
            <p class="message problem" role="alert">{$t("transcription.failed", { details: addon.activity.message })}</p>
          {/if}
          {#if addon?.needsReinstall}
            <p class="message problem" role="alert">{$t("addons.manager.status.needs-reinstall")}</p>
          {:else if addon?.installedProblem}
            <p class="message problem" role="alert">{$t("transcription.failed", { details: addon.installedProblem })}</p>
          {/if}
          {#if addon?.release && addon.platformSupported}
            <button type="button" class="primary wide" disabled={addonBusy || transferring} onclick={() => void download()}>
              {$t("transcription.downloadAddon", { size: megabytes(addon.release.sizeBytes) })}
            </button>
          {:else}
            <p class="hint">{$t("transcription.addonUnavailable")}</p>
          {/if}
        {/if}
      </section>
    {:else if kind && !busy}
      <label class="result">
        <span class="section-label">{kind === "math" ? $t("transcription.mathLabel") : $t("transcription.textLabel")}</span>
        <textarea bind:this={field} bind:value rows={kind === "math" ? 3 : 4} spellcheck={kind === "text"} class:code={kind === "math"}></textarea>
      </label>
      {#if rawLatex}
        <details class="latex">
          <summary>{$t("transcription.rawLatex")}</summary>
          <code>{rawLatex}</code>
        </details>
      {/if}
      <p class="hint">{kind === "math" ? $t("transcription.mathHint") : $t("transcription.keepsInk")}</p>
    {/if}

    {#if addon?.installed}
      <details class="addon">
        <summary>{$t("transcription.addonInstalled", { version: addon.installed })}</summary>
        <p class="hint">{$t("transcription.addonDetail")}</p>
        <button type="button" class="control" disabled={addonBusy || busy} onclick={() => confirmingRemoval = true}>{$t("transcription.removeAddon")}</button>
      </details>
    {/if}
  </div>

  <footer>
    <button type="button" class="control" onclick={onClose}>{$t("transcription.cancel")}</button>
    <button type="button" class="primary" disabled={!kind || busy || offering || !value.trim()} onclick={insert}>{$t("transcription.insert")}</button>
  </footer>
</div>

{#if confirmingRemoval && recognizerAddon}
  <ConfirmDialog title={$t("addons.manager.confirmTitle", { name: $t(`${recognizerAddon.i18nPrefix}.label`) })}
    detail={$t("addons.manager.confirmDetail")} confirmLabel={$t("addons.manager.remove")}
    onCancel={() => confirmingRemoval = false}
    onConfirm={() => { confirmingRemoval = false; void removeAddon(); }} />
{/if}

<style>
  .transcription {
    position: absolute;
    top: 70px;
    right: 16px;
    z-index: 30;
    display: flex;
    width: 296px;
    max-width: calc(100% - 32px);
    max-height: calc(100% - 116px);
    flex-direction: column;
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    background: var(--panel);
    box-shadow: var(--shadow-panel);
    color: var(--text);
    font-family: var(--font-ui);
    font-size: var(--text-md);
    line-height: 1.4;
  }
  .transcription:focus-visible { outline: none; }

  header { display: flex; height: var(--control); flex: none; align-items: center; padding: 0 8px 0 12px; border-bottom: 1px solid var(--edge-soft); gap: 8px; }
  h2 { flex: 1; margin: 0; font-size: var(--text-lg); font-weight: 600; line-height: 1.25; }
  .chrome { display: grid; width: var(--control-dense); height: var(--control-dense); padding: 0; border: 0; border-radius: var(--radius); background: transparent; color: var(--muted); cursor: pointer; place-items: center; transition: background var(--fast) ease, color var(--fast) ease; }
  .chrome:hover { background: var(--wash); color: var(--text); }
  .chrome:active { background: var(--wash-strong); }
  .chrome svg { width: var(--icon-dense); height: var(--icon-dense); fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: var(--stroke-dense); }

  .body { display: flex; min-height: 0; flex-direction: column; padding: 12px; gap: 12px; overflow-y: auto; }
  section { display: flex; flex-direction: column; gap: 8px; }
  .section-label { color: var(--muted); font-size: var(--text-sm); font-weight: 600; }

  /* The writer's own ink on paper: content, so its colour is not a UI token. */
  .original { display: block; width: 100%; height: 88px; flex: none; border-radius: 3px; background: var(--paper); }
  .original polyline { fill: none; stroke: #1d2430; stroke-linecap: round; stroke-linejoin: round; }

  /* Option grid: tiles with an icon and a label keep a 1px edge at rest; the chosen one is selected. */
  .kinds { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .kinds button { display: flex; min-height: var(--control-touch); align-items: center; padding: 0 12px; border: 1px solid var(--edge); border-radius: var(--radius); background: transparent; color: var(--muted); cursor: pointer; gap: 8px; font: inherit; font-size: var(--text-md); transition: background var(--fast) ease, border-color var(--fast) ease, color var(--fast) ease; }
  .kinds button:hover:not(:disabled) { background: var(--wash); color: var(--text); }
  .kinds button:active:not(:disabled) { background: var(--wash-strong); }
  .kinds button.current { border-color: var(--accent-edge); background: var(--accent-fill); color: var(--text); font-weight: 600; }
  .kinds svg { width: var(--icon-dense); height: var(--icon-dense); flex: none; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: var(--stroke-dense); }

  /* A field: dark-scheme so the native list matches the panel, with its own chevron. */
  .language { display: flex; flex-direction: column; gap: 4px; }
  .select-shell { position: relative; display: flex; }
  select { width: 100%; min-width: 0; height: var(--control); padding: 0 32px 0 12px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); color: var(--text); color-scheme: dark; font: inherit; font-size: var(--text-md); text-overflow: ellipsis; appearance: none; cursor: pointer; transition: border-color var(--fast) ease; }
  select:hover:not(:disabled) { border-color: var(--edge-strong); }
  option { background: var(--panel); color: var(--text); }
  .select-shell svg { position: absolute; top: 50%; right: 8px; width: var(--icon-dense); height: var(--icon-dense); transform: translateY(-50%); fill: none; stroke: var(--muted); stroke-width: var(--stroke-dense); stroke-linecap: round; stroke-linejoin: round; pointer-events: none; }

  .result { display: flex; flex-direction: column; gap: 4px; }
  textarea { width: 100%; padding: 8px 12px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); color: var(--text); font: inherit; font-size: var(--text-md); line-height: 1.4; resize: vertical; caret-color: var(--blueprint-light); transition: border-color var(--fast) ease; }
  textarea:hover { border-color: var(--edge-strong); }
  textarea.code { font-family: var(--font-code); }
  textarea::selection, .latex code::selection { background: var(--accent-select); }

  .message, .hint, .lead { margin: 0; font-size: var(--text-sm); line-height: 1.4; }
  .message { color: var(--muted); }
  .message.problem { color: var(--oxide-text); }
  .hint { color: var(--quiet); }
  .lead { color: var(--text); font-size: var(--text-md); }

  .offer { gap: 12px; }
  .facts { display: grid; grid-template-columns: auto 1fr; margin: 0; gap: 4px 12px; font-size: var(--text-sm); }
  .facts dt { color: var(--quiet); }
  .facts dd { margin: 0; color: var(--muted); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .figure { color: var(--muted); font-size: var(--text-sm); font-variant-numeric: tabular-nums; }

  .progress { position: relative; height: 6px; overflow: hidden; border-radius: 3px; background: var(--wash); }
  /* Scaled, not resized: the bar updates while the writer may be drawing, and a transform never
     makes the page lay out again. */
  .progress span { position: absolute; inset: 0; border-radius: inherit; background: var(--blueprint); transform: scaleX(0); transform-origin: left; transition: transform 200ms ease-out; }
  /* Installing has no measurable progress: the full bar breathes only while work runs. */
  .progress.indeterminate span { transform: none; animation: working 1.2s ease-in-out infinite; }
  @keyframes working { 0%, 100% { opacity: 0.35; } 50% { opacity: 1; } }

  details { font-size: var(--text-sm); }
  summary { color: var(--quiet); cursor: pointer; }
  summary:hover { color: var(--muted); }
  details[open] { display: flex; flex-direction: column; gap: 8px; }
  details[open] > .control { align-self: flex-start; }
  .latex code { display: block; padding: 8px; border-radius: var(--radius); background: var(--inset); color: var(--muted); font-family: var(--font-code); overflow-wrap: anywhere; user-select: text; }
  .addon { padding-top: 12px; border-top: 1px solid var(--edge-soft); }

  footer { display: flex; flex: none; justify-content: flex-end; padding: 12px; border-top: 1px solid var(--edge-soft); gap: 8px; }

  .control, .primary { display: inline-flex; min-height: var(--control); align-items: center; justify-content: center; padding: 0 16px; border: 0; border-radius: var(--radius); font: inherit; font-size: var(--text-md); white-space: nowrap; cursor: pointer; touch-action: manipulation; transition: background var(--fast) ease; }
  .control { background: var(--wash); color: var(--text); }
  .control:hover:not(:disabled) { background: var(--wash-strong); }
  .primary { background: var(--blueprint); color: var(--on-accent); font-weight: 600; }
  .primary:hover:not(:disabled) { background: var(--blueprint-hover); }
  .primary.wide { width: 100%; }

  button:disabled { cursor: default; opacity: var(--disabled); }
  button:focus-visible, select:focus-visible, textarea:focus-visible, summary:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }

  @media (pointer: coarse) {
    .control, .primary { min-height: var(--control-touch); }
  }
  @media (prefers-reduced-motion: reduce) {
    .kinds button, .progress span, .control, .primary, .chrome, select, textarea { transition: none; }
    .progress.indeterminate span { animation: none; opacity: 0.7; }
  }
</style>
