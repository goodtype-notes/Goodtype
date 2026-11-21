<script lang="ts">
  import { onMount, tick } from "svelte";
  import { t, locale, notice, renderMessage, type UiText } from "../i18n";
  import { errorDetails } from "../ipc/errors";
  import {
    addonText,
    cancelAddonCommand,
    dismissAddonProposal,
    editAddonProposal,
    invokeAddonSelection,
    type AddonReview,
    type SelectionCommand,
  } from "../ipc/addons";
  import { addonProblem } from "./addonCommand";

  /**
   * One explicit run of an addon's ink-selection command (ADR 0045). Rust captures the committed
   * selection, runs the addon and returns a proposal for review. A direct apply happens only when
   * Rust confirms both opt-ins, and is then shown with Undo. Ink is never changed.
   */
  type Props = {
    root: string;
    pageId: string;
    strokeIds: string[];
    command: SelectionCommand;
    insert: (review: AddonReview, direct: boolean) => Promise<boolean>;
    undo: () => void;
    onClose: () => void;
  };
  let { root, pageId, strokeIds, command, insert, undo, onClose }: Props = $props();

  type Phase = "running" | "review" | "applied" | "failed";
  let phase = $state<Phase>("running");
  let review = $state<AddonReview | null>(null);
  let sources = $state<string[]>([]);
  let message = $state<UiText | null>(null);
  let busy = $state(false);
  let field = $state<HTMLTextAreaElement>();
  let panel = $state<HTMLElement>();
  const requestId = crypto.randomUUID();
  let disposed = false;

  const name = $derived(addonText(review?.addon ?? command.label, $locale));
  const label = $derived(addonText(command.label, $locale));

  function fail(error: unknown) {
    phase = review ? "review" : "failed";
    message = notice(addonProblem(errorDetails(error)), { addon: label });
  }

  async function run() {
    phase = "running";
    message = null;
    review = null;
    try {
      const result = await invokeAddonSelection({ root, pageId, strokeIds, addonId: command.addonId, command: command.command, requestId, reviewFirst: false });
      if (disposed) { void dismissAddonProposal(result.review.id).catch(() => {}); return; }
      review = result.review;
      sources = result.review.items.length ? result.review.items.map((item) => item.source) : [""];
      if (result.direct && await insert(result.review, true)) {
        phase = "applied";
        await tick();
        panel?.querySelector<HTMLButtonElement>(".primary")?.focus();
        return;
      }
      phase = "review";
      if (result.review.latex !== null) message = notice("addonCommand.unsupported", { addon: name });
      await tick();
      field?.focus();
    } catch (error) {
      if (!disposed) fail(error);
    }
  }

  async function accept() {
    if (!review || busy || !sources.every((source) => source.trim())) return;
    busy = true;
    message = null;
    try {
      let current = review;
      const edited = current.latex !== null || sources.some((source, index) => source !== current.items[index]?.source);
      // A correction becomes a new proposal version, compiled and checked again by Rust.
      if (edited) current = review = await editAddonProposal(current.id, current.version, sources);
      await insert(current, false);
      onClose();
    } catch (error) {
      fail(error);
    } finally {
      busy = false;
    }
  }

  function close() {
    if (phase === "running") void cancelAddonCommand(requestId).catch(() => {});
    if (review && phase !== "applied") void dismissAddonProposal(review.id).catch(() => {});
    onClose();
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && phase === "review") {
      event.preventDefault();
      void accept();
    }
  }

  onMount(() => {
    void run();
    return () => {
      disposed = true;
      if (phase === "running") void cancelAddonCommand(requestId).catch(() => {});
    };
  });
</script>

<div class="addon-command" role="dialog" aria-modal="false" aria-labelledby="addon-command-title" tabindex="-1" bind:this={panel} onkeydown={keydown}>
  <header>
    <h2 id="addon-command-title">{label}</h2>
    <button type="button" class="chrome" title={$t("addonCommand.close")} aria-label={$t("addonCommand.close")} onclick={close}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"></path></svg>
    </button>
  </header>

  <div class="body">
    {#if phase === "running"}
      <p class="lead" role="status">{$t("addonCommand.running", { addon: name })}</p>
      <p class="hint">{$t("addonCommand.scope", { addon: name, count: strokeIds.length })}</p>
      <div class="progress" role="progressbar" aria-label={$t("addonCommand.running", { addon: name })}><span></span></div>
    {:else if phase === "applied"}
      <p class="lead" role="status">{$t("addonCommand.applied", { addon: name })}</p>
    {:else if phase === "review" && review}
      <p class="hint">{$t("addonCommand.proposedBy", { addon: name })}</p>
      {#each sources as _, index (index)}
        {#if review.items[index] && sources[index] === review.items[index].source}
          <img class="preview" alt={$t("addonCommand.preview")} src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(review.items[index].svg)}`} />
        {/if}
        <label class="result">
          <span class="section-label">{$t("addonCommand.source")}</span>
          {#if index === 0}
            <textarea bind:this={field} bind:value={sources[index]} rows="3" spellcheck="false"></textarea>
          {:else}
            <textarea bind:value={sources[index]} rows="3" spellcheck="false"></textarea>
          {/if}
        </label>
      {/each}
      {#if review.latex !== null}
        <details class="latex" open>
          <summary>{$t("addonCommand.latex")}</summary>
          <code>{review.latex}</code>
        </details>
      {/if}
      <p class="hint">{$t("addonCommand.keepsInk")}</p>
    {/if}
    {#if message}
      <p class="message problem" role="alert">{renderMessage(message, $t)}</p>
    {/if}
  </div>

  <footer>
    {#if phase === "running"}
      <button type="button" class="control" onclick={close}>{$t("addonCommand.cancel")}</button>
    {:else if phase === "applied"}
      <button type="button" class="control" onclick={() => { undo(); onClose(); }}>{$t("addonCommand.undo")}</button>
      <button type="button" class="primary" onclick={onClose}>{$t("addonCommand.done")}</button>
    {:else if phase === "failed"}
      <button type="button" class="control" onclick={close}>{$t("addonCommand.close")}</button>
      <button type="button" class="primary" onclick={() => void run()}>{$t("addonCommand.retry")}</button>
    {:else}
      <button type="button" class="control" disabled={busy} onclick={close}>{$t("addonCommand.dismiss")}</button>
      <button type="button" class="primary" disabled={busy || !sources.every((source) => source.trim())} onclick={() => void accept()}>{$t("addonCommand.insert")}</button>
    {/if}
  </footer>
</div>

<style>
  .addon-command { position: absolute; top: 70px; right: 16px; z-index: 30; display: flex; width: 296px; max-width: calc(100% - 32px); max-height: calc(100% - 116px); flex-direction: column; border: 1px solid var(--edge); border-radius: var(--radius-lg); background: var(--panel); box-shadow: var(--shadow-panel); color: var(--text); font-size: var(--text-md); }
  .addon-command:focus-visible { outline: none; }
  header { display: flex; height: var(--control); flex: none; align-items: center; padding: 0 6px 0 12px; border-bottom: 1px solid var(--edge-soft); gap: 8px; }
  h2 { flex: 1; margin: 0; overflow: hidden; font-size: var(--text-md); font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
  .chrome { display: grid; width: var(--control-dense); height: var(--control-dense); padding: 0; border: 0; border-radius: var(--radius); background: transparent; color: var(--muted); cursor: pointer; place-items: center; }
  .chrome:hover { background: var(--wash); color: var(--text); }
  .chrome svg { width: var(--icon-dense); height: var(--icon-dense); fill: none; stroke: currentColor; stroke-linecap: round; stroke-width: var(--stroke-dense); }
  .body { display: flex; min-height: 0; flex-direction: column; padding: 12px; gap: 11px; overflow-y: auto; }
  .section-label { color: var(--muted); font-size: var(--text-sm); }
  .preview { display: block; max-width: 100%; max-height: 120px; padding: 6px; border-radius: var(--radius); background: var(--paper); object-fit: contain; }
  .result { display: flex; flex-direction: column; gap: 7px; }
  textarea { width: 100%; padding: 7px 9px; border: 1px solid var(--edge-soft); border-radius: var(--radius); background: rgb(0 0 0 / 22%); color: var(--text); font-family: "Cascadia Mono", Consolas, monospace; font-size: var(--text-md); line-height: 1.45; resize: vertical; caret-color: var(--blueprint-light); }
  .message, .hint, .lead { margin: 0; font-size: var(--text-sm); line-height: 1.45; }
  .message.problem { color: var(--oxide); }
  .hint { color: var(--quiet); }
  .lead { color: var(--text); font-size: var(--text-md); }
  .progress { position: relative; height: 6px; overflow: hidden; border-radius: 3px; background: var(--wash); }
  .progress span { position: absolute; top: 0; bottom: 0; left: 0; width: 30%; border-radius: inherit; background: var(--blueprint); animation: travel 1.3s ease-in-out infinite; }
  @keyframes travel { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
  details { font-size: var(--text-sm); }
  summary { color: var(--quiet); cursor: pointer; }
  .latex code { display: block; margin-top: 6px; padding: 6px 8px; border-radius: var(--radius); background: rgb(0 0 0 / 22%); color: var(--muted); font-family: "Cascadia Mono", Consolas, monospace; overflow-wrap: anywhere; user-select: text; }
  footer { display: flex; flex: none; justify-content: flex-end; padding: 10px 12px; border-top: 1px solid var(--edge-soft); gap: 8px; }
  .control, .primary { display: inline-flex; min-height: var(--control); align-items: center; justify-content: center; padding: 0 12px; border: 0; border-radius: var(--radius); font: inherit; font-size: var(--text-md); white-space: nowrap; cursor: pointer; touch-action: manipulation; }
  .control { background: var(--wash); color: var(--text); }
  .control:hover:not(:disabled) { background: var(--wash-strong); }
  .primary { background: var(--blueprint); color: var(--on-accent); font-weight: 600; }
  button:disabled { cursor: not-allowed; opacity: var(--disabled); }
  button:focus-visible, textarea:focus-visible, summary:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  @media (pointer: coarse) { .control, .primary { min-height: var(--control-touch); } }
  @media (prefers-reduced-motion: reduce) { .progress span { animation: none; } }
</style>
