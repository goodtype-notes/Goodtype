<script lang="ts">
  import { t, renderMessage, type UiText } from "../i18n";
  let { status, transactionFailed, pageNumber, pageCount, zoom, saveNeedsAttention, savePending, overviewOpen, overviewId, onZoom, onToggleOverview, onSave }: {
    /** Still accepted so callers compile; the slim bar no longer shows the active tool. */
    toolLabel?: string; toolDetail?: string;
    status: UiText; transactionFailed: boolean;
    pageNumber: number; pageCount: number; zoom: number; saveNeedsAttention: boolean; savePending: boolean;
    overviewOpen: boolean; overviewId: string;
    onZoom: (zoom: number) => void; onToggleOverview: () => void; onSave: () => void;
  } = $props();

  /**
   * The status line is shared by routine feedback ("Pen 1 active", "Updated Typst source; saved
   * revision 42") and by problems. Only problems earn space in the slim bar.
   *
   * A problem is a failed transaction, or a message whose catalog key names one: an `errors.*`
   * key, or a key containing fail, error, could not, cannot, unable, missing, or a refused limit
   * ("at most", "at least"), or a "…safe but…" warning. Plain strings carry no key to judge, so
   * they stay visible rather than risk hiding a failure.
   */
  const PROBLEM_KEY = /^errors\.|fail|error|could|cannot|unable|missing|atmost|atleast|safebut|but(its|could)|stoppedon|policy/i;
  function isProblem(value: UiText): boolean {
    if (typeof value === "string") return value.trim() !== "";
    return PROBLEM_KEY.test(value.key);
  }
  const problem = $derived(transactionFailed || isProblem(status));
  const problemText = $derived(problem ? renderMessage(status, $t) : "");
</script>

  <footer class="status-strip" data-ui="solid">
    {#if problemText}
      <div class="problem" class:failure={transactionFailed} role="status" title={problemText}>{problemText}</div>
    {:else}
      <div class="spacer"></div>
    {/if}
    <!-- The page count is where you look to learn where you are, so it also opens every page. -->
    <button
      type="button"
      id="page-overview-toggle"
      class="page-count"
      class:open={overviewOpen}
      title={$t("pageOverview.show")}
      aria-expanded={overviewOpen}
      aria-controls={overviewId}
      onclick={onToggleOverview}
    >{$t("workspace.pageOf", { "value2": pageNumber, "value4": pageCount })}</button>
    <span class="footer-divider"></span>
    <!-- Zoom reads and is changed in one place. -->
    <span class="zoom-group">
      <button type="button" aria-label={$t("workspace.zoomOut")} onclick={() => onZoom(zoom / 1.4)}>−</button>
      <button type="button" class="zoom-value" aria-label={$t("workspace.resetZoomTo")} title={$t("workspace.resetZoom")} onclick={() => onZoom(1)}>{Math.round(zoom * 100)}%</button>
      <button type="button" aria-label={$t("workspace.zoomIn")} onclick={() => onZoom(zoom * 1.4)}>+</button>
    </span>
    <span class="footer-divider"></span><button type="button" class:failure={saveNeedsAttention} class="local-state" title={$t("workspace.saveNow")} onclick={onSave}>{saveNeedsAttention ? $t("workspace.needsAttention") : savePending ? $t("workspace.localSaving") : $t("workspace.localSaved")}</button>
  </footer>


<style>
  button { border: 0; font: inherit; }
  button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  .status-strip {
    z-index: 30;
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    border-top: 1px solid var(--edge-soft);
    background: var(--panel);
    color: var(--muted);
    font-size: var(--text-sm);
  }

  .spacer { flex: 1; }
  .problem { overflow: hidden; flex: 1; color: var(--amber-text); text-overflow: ellipsis; white-space: nowrap; }
  .problem.failure, .status-strip .local-state.failure { color: var(--oxide-text); }
  .footer-divider { width: 1px; height: 16px; flex: none; background: var(--edge); }
  .status-strip button { padding: 4px 8px; border-radius: var(--radius); background: transparent; color: var(--muted); cursor: pointer; transition: background var(--fast) ease, color var(--fast) ease; }
  .status-strip button:hover { background: var(--wash); color: var(--text); }
  .status-strip button:active { background: var(--wash-strong); }
  .local-state { flex: none; }

  .page-count { font-variant-numeric: tabular-nums; }
  .status-strip button.page-count.open { background: var(--accent-fill); color: var(--text); }

  .zoom-group { display: flex; align-items: center; gap: 2px; }
  .zoom-group .zoom-value { min-width: 48px; font-variant-numeric: tabular-nums; text-align: center; }

  @media (prefers-reduced-motion: reduce) {
    .status-strip button { transition: none; }
  }
</style>
