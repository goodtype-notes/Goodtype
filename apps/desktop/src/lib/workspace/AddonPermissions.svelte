<script lang="ts">
  import { t } from "../i18n";
  import type { AddonPermission } from "../ipc/addons";

  let { permissions, update = false, approval = false, install = false }: { permissions: AddonPermission[]; update?: boolean; approval?: boolean; install?: boolean } = $props();
</script>

<div class="permissions" data-ui="solid">
  <p class="label">{$t(approval ? "addons.manager.approvalPermissions" : update ? "addons.manager.updatePermissions" : install ? "addons.store.installPermissions" : "addons.manager.permissions")}</p>
  {#if permissions.length}
    <ul>
      {#each permissions as permission (permission)}
        <li>{$t(`addons.manager.permission.${permission}`)}</li>
      {/each}
    </ul>
  {:else}
    <p class="hint">{$t("addons.manager.noPermissions")}</p>
  {/if}
</div>

<style>
  /* No box of its own: the parent decides the layout, this only carries the solid tokens. */
  .permissions { display: contents; }
  .label { margin: 0; color: var(--text); font-size: var(--text-md); font-weight: 600; line-height: 1.4; }
  ul { margin: 4px 0 8px; padding-inline-start: 20px; color: var(--text); font-size: var(--text-md); line-height: 1.5; }
  .hint { margin: 0; color: var(--muted); font-size: var(--text-sm); line-height: 1.4; }
</style>
