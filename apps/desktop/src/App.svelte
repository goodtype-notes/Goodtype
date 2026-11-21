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

<NotebookWorkspace />

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
  .addon-notice.visible {
    position: fixed;
    z-index: 50;
    inset: auto 1rem 2rem auto;
    display: flex;
    align-items: center;
    gap: .75rem;
    padding: .5rem .75rem;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--edge);
    border-radius: var(--radius);
  }
  .addon-notice button { color: inherit; background: transparent; border: 1px solid var(--edge); }

  /* One scrollbar for the whole app. The platform's own is a 17px tray with stepper arrows at
     both ends, which is wider than some of the controls it sits beside and belongs to a different
     decade of Windows than the rest of this chrome. Overlay-thin, and it fades into the panel it
     scrolls rather than framing it. */
  :global(*) {
    scrollbar-width: thin;
    scrollbar-color: rgb(255 255 255 / 16%) transparent;
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
    background: rgb(255 255 255 / 16%);
    background-clip: padding-box;
  }

  :global(::-webkit-scrollbar-thumb:hover) {
    background: rgb(255 255 255 / 30%);
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
