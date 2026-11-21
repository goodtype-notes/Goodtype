<script lang="ts">
  import { t } from "../i18n";
  import type { AddonPermission } from "../ipc/addons";

  let { permissions, update = false }: { permissions: AddonPermission[]; update?: boolean } = $props();
</script>

<p class="label">{$t(update ? "addons.manager.updatePermissions" : "addons.manager.permissions")}</p>
{#if permissions.length}
  <ul>
    {#each permissions as permission (permission)}
      <li>{$t(`addons.manager.permission.${permission}`)}</li>
    {/each}
  </ul>
{:else}
  <p class="hint">{$t("addons.manager.noPermissions")}</p>
{/if}

<style>
  .label { font-weight: 600; }
  ul { padding-inline-start: 20px; margin: 4px 0 8px; line-height: 1.5; }
</style>
