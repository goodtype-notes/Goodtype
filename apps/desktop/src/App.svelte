<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { checkAddonRegistry } from "./lib/ipc/addons";
  import { t, notice, renderMessage, type UiText } from "./lib/i18n";
  import NotebookWorkspace from "./lib/notebook/NotebookWorkspace.svelte";

  let addonNotice = $state<UiText>();
  onMount(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const subscription = listen("addon-update-available", () => {
      addonNotice = notice("addons.manager.updateAvailable");
    });
    void subscription.catch(() => {}).then(() => checkAddonRegistry("startup")).catch(() => {});
    return () => { void subscription.then(unlisten => unlisten()).catch(() => {}); };
  });
</script>

<NotebookWorkspace onAppNotice={message => { addonNotice = message; }} />

<div class="addon-notice" class:visible={addonNotice}>
  <span role="status">{addonNotice ? renderMessage(addonNotice, $t) : ""}</span>
  {#if addonNotice}
    <button type="button" onclick={() => addonNotice = undefined}>{$t("addons.manager.dismissNotice")}</button>
  {/if}
</div>

<svelte:head>
  <title>Goodtype</title>
</svelte:head>

<style>
  /* A floating notice: the menu recipe's surface, one quiet secondary action. It sits above the
     status bar so it never covers the save state. */
  .addon-notice.visible {
    position: fixed;
    z-index: 50;
    inset: auto 16px 48px auto;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 6px 6px 16px;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-menu);
    font: var(--text-md) var(--font-ui);
  }
  .addon-notice button { min-height: var(--control); padding: 0 14px; border: 0; border-radius: var(--radius); color: var(--text); background: var(--wash); font: inherit; cursor: pointer; }
  .addon-notice button:hover { background: var(--wash-strong); }
  .addon-notice button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }

  /* One scrollbar for the whole app. The platform's own is a 17px tray with stepper arrows at
     both ends, which is wider than some of the controls it sits beside and belongs to a different
     decade of Windows than the rest of this chrome. Thin and solid, from the ladder. */
  :global(*) {
    scrollbar-width: thin;
    scrollbar-color: var(--edge-strong) transparent;
  }

  :global(::-webkit-scrollbar) {
    width: 8px;
    height: 8px;
  }

  :global(::-webkit-scrollbar-track),
  :global(::-webkit-scrollbar-corner) {
    background: transparent;
  }

  :global(::-webkit-scrollbar-thumb) {
    border: 2px solid transparent;
    border-radius: 999px;
    background: var(--edge-strong);
    background-clip: padding-box;
  }

  :global(::-webkit-scrollbar-thumb:hover) {
    background: var(--muted);
    background-clip: padding-box;
  }

  :global(html),
  :global(body),
  :global(#app) {
    width: 100%;
    height: 100%;
    min-height: 100%;
    margin: 0;
    overflow: hidden;
  }
</style>
