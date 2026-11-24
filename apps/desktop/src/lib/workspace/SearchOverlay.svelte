<script lang="ts">
  import { t } from "../i18n";
  import { errorDetails } from "../ipc/errors";
  import { searchNotebook } from "../ipc/notebook";
  import type { SearchHit } from "../settings";

  let {
    root,
    tauriAvailable,
    onNavigate,
    onClose,
  }: {
    root: string;
    tauriAvailable: boolean;
    onNavigate: (hit: SearchHit) => void;
    onClose: () => void;
  } = $props();

  let query = $state("");
  let hits = $state<SearchHit[]>([]);
  let searched = $state(false);
  let failure = $state("");
  let inputElement = $state<HTMLInputElement>();
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchGeneration = 0;

  $effect(() => {
    inputElement?.focus();
    return () => {
      searchGeneration += 1;
      if (searchTimer) clearTimeout(searchTimer);
    };
  });

  function scheduleSearch() {
    searchGeneration += 1;
    hits = [];
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(run, 200);
  }

  async function run() {
    const generation = ++searchGeneration;
    const requestedRoot = root;
    const trimmed = query.trim();
    const current = () => generation === searchGeneration && root === requestedRoot && query.trim() === trimmed;
    if (!trimmed || !tauriAvailable) {
      hits = [];
      searched = false;
      failure = "";
      return;
    }
    try {
      const result = await searchNotebook(requestedRoot, trimmed);
      if (!current()) return;
      hits = result;
      searched = true;
      failure = "";
    } catch (error) {
      if (!current()) return;
      failure = errorDetails(error);
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    } else if (event.key === "Enter" && hits.length > 0) {
      event.preventDefault();
      onNavigate(hits[0]);
    }
  }
</script>

<!-- Notebook-scoped Typst source search (Phase 2 §3.7): typed content only, no OCR. -->
<section class="search" data-ui="solid" role="search" aria-label={$t("workspace.searchTypedNotebookContent")}>
  <div class="field">
    <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="7"></circle><path d="m20 20-4-4"></path></svg>
    <input
      bind:this={inputElement}
      bind:value={query}
      type="search"
      placeholder={$t("workspace.searchTypedContent2")}
      aria-label={$t("workspace.searchTypedNotebookContent")}
      oninput={scheduleSearch}
      onkeydown={keydown}
    />
    <button type="button" aria-label={$t("workspace.closeSearch")} onclick={onClose}>×</button>
  </div>
  {#if failure}
    <p class="state">{$t("errors.operation", { details: failure })}</p>
  {:else if searched && hits.length === 0}
    <p class="state">{$t("workspace.noTypedContentMatches")}</p>
  {:else if hits.length > 0}
    <ol aria-label={$t("workspace.searchResults")}>
      {#each hits as hit (hit.pageId + hit.objectId)}
        <li>
          <button type="button" onclick={() => onNavigate(hit)}>
            <span class="page">{$t("workspace.p", { "pageNumber": hit.pageNumber })}</span>
            <span class="excerpt">{hit.excerpt}</span>
          </button>
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .search {
    position: absolute;
    top: 12px;
    right: 16px;
    z-index: 30;
    width: min(380px, calc(100% - 32px));
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    background: var(--panel);
    box-shadow: var(--shadow-menu);
  }

  .field { display: flex; align-items: center; gap: 8px; padding: 4px 4px 4px 12px; }

  .field svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    flex: none;
    fill: none;
    stroke: var(--muted);
    stroke-linecap: round;
    stroke-width: var(--stroke-dense);
  }

  input {
    flex: 1;
    min-width: 0;
    height: var(--control);
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    outline: none;
  }
  input::placeholder { color: var(--quiet); }

  .field > button {
    flex: none;
    width: var(--control-dense);
    height: var(--control-dense);
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    font-size: var(--text-lg);
    line-height: 1;
    cursor: pointer;
  }
  .field > button:hover { background: var(--wash); color: var(--text); }
  .field > button:active { background: var(--wash-strong); }
  .field > button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }

  .state {
    margin: 0;
    padding: 12px;
    border-top: 1px solid var(--edge-soft);
    color: var(--muted);
    font-size: var(--text-md);
  }

  ol {
    max-height: 280px;
    overflow: auto;
    margin: 0;
    padding: 4px;
    border-top: 1px solid var(--edge-soft);
    list-style: none;
  }

  li button {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: var(--control);
    padding: 0 12px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--text-md);
    text-align: left;
    cursor: pointer;
  }
  li button:hover { background: var(--wash); }
  li button:active { background: var(--wash-strong); }
  li button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: -2px; }

  /* On a hovered row (--wash) --quiet fails contrast, so the page number is --muted. */
  .page { flex: none; color: var(--muted); font-size: var(--text-sm); font-variant-numeric: tabular-nums; }
  .excerpt { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

</style>
