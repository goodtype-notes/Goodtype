<script lang="ts">
  import { t } from "../i18n";
  import { approveAddon, type AddonApproval } from "../ipc/addons";
  import { cancelImporter } from "../ipc/importer";
  import AddonPermissions from "./AddonPermissions.svelte";

  let { id, approval, refresh }: {
    id: string;
    approval: AddonApproval;
    refresh: () => void | Promise<void>;
  } = $props();
  let pending = $state(false);
  let failed = $state(false);

  async function decide(approve: boolean) {
    pending = true;
    failed = false;
    try {
      if (approve) await approveAddon(id, approval.token);
      else await cancelImporter(id);
      await refresh();
    } catch {
      failed = true;
    } finally {
      pending = false;
    }
  }
</script>

<div class="approval" data-ui="solid">
  <p>{$t("addons.manager.approvalDetail", { version: approval.version })}</p>
  <AddonPermissions permissions={approval.permissions} approval />
  <button type="button" class="primary" disabled={pending} onclick={() => void decide(true)}>{$t("addons.manager.approve")}</button>
  <button type="button" class="control" disabled={pending} onclick={() => void decide(false)}>{$t("library.cancel")}</button>
  {#if failed}<p role="alert" class="problem">{$t("addons.manager.actionFailed")}</p>{/if}
</div>

<style>
  /* No box of its own: the parent decides the layout, this only carries the solid tokens. */
  .approval { display: contents; }
  .approval > p { margin: 0; line-height: 1.4; }
  .problem { color: var(--oxide-text); }
</style>
