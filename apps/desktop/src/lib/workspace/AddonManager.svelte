<script lang="ts">
  import { t, locale, formatNumber } from "../i18n";
  import { removeAddon, setAddonDirectApply, setAddonEnabled, type ManagedAddon } from "../ipc/addons";
  import { cancelImporter, checkImporterUpdate, installImporter, megabytes, updateImporter } from "../ipc/importer";
  import AddonApproval from "./AddonApproval.svelte";
  import AddonPermissions from "./AddonPermissions.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import Dialog from "./Dialog.svelte";

  let { addon, refresh, onClose }: {
    addon: ManagedAddon;
    refresh: () => Promise<void>;
    onClose: () => void;
  } = $props();

  let pending = $state(false);
  let confirming = $state(false);
  let problem = $state<"actionFailed" | "policyFailed" | "removeFailed" | null>(null);
  const name = $derived($locale === "de" ? addon.name.de : addon.name.en);
  const busy = $derived(["checking", "downloading", "installing", "awaitingApproval", "importing", "publishing"].includes(addon.activity));
  const release = $derived(addon.update ?? addon.release);

  async function run(action: (id: string) => Promise<void>, failure: typeof problem = "actionFailed") {
    pending = true;
    problem = null;
    try {
      await action(addon.id);
      await refresh();
    } catch {
      problem = failure;
    } finally {
      pending = false;
    }
  }
</script>

<Dialog title={name} size="lg" closeButton dismissible={!confirming} {onClose}>
  <dl class="facts">
    <dt>{$t("addons.manager.statusLabel")}</dt>
    <dd>{$t(`addons.manager.status.${addon.status}`, { expected: addon.expectedApi, found: addon.foundApi || $t("addons.manager.unknownVersion") })}</dd>
    {#if addon.installed}
      <dt>{$t("addons.manager.installed")}</dt><dd>{addon.installed}</dd>
    {/if}
    {#if release}
      <dt>{$t("addons.manager.download")}</dt>
      <dd>{$t("addons.manager.release", { version: release.version, size: megabytes(release.sizeBytes) })}</dd>
    {/if}
    {#if addon.formats.length}
      <dt>{$t("addons.manager.formats")}</dt><dd>{addon.formats.join(", ")}</dd>
    {/if}
    <dt>{$t("addons.manager.limitsLabel")}</dt>
    <dd>{$t("addons.manager.limits", { memory: megabytes(addon.memoryBytes), seconds: formatNumber(addon.timeoutSeconds), output: megabytes(addon.outputBytes) })}</dd>
  </dl>

  <section>
    <AddonPermissions permissions={addon.permissions} />
    {#if !addon.approval && addon.update && addon.catalogPermissions.some(permission => !addon.permissions.includes(permission))}
      <AddonPermissions permissions={addon.catalogPermissions} update />
    {/if}
    {#if addon.status === "disabled"}
      <p class="hint">{$t("addons.manager.reviewPermissions")}</p>
    {/if}
    <p class="hint">{$t("addons.manager.network")}</p>
  </section>

  {#if addon.installed && (addon.status === "enabled" || addon.status === "disabled")}
    <label class="direct">
      <input type="checkbox" checked={addon.directApply} disabled={busy || pending}
        aria-describedby="addon-direct-hint"
        onchange={event => {
          const allowed = event.currentTarget.checked;
          event.currentTarget.checked = addon.directApply;
          void run(id => setAddonDirectApply(id, allowed), "policyFailed");
        }} />
      {$t("addons.manager.direct")}
    </label>
    <p class="hint" id="addon-direct-hint">{$t("addons.manager.directHint")}</p>
  {/if}

  {#if addon.activity !== "idle"}
    <p role="status">{$t(`addons.manager.activity.${addon.activity}`)}</p>
    {#if addon.progress}
      <progress max={addon.progress[1]} value={addon.progress[0]} aria-label={$t("addons.manager.activity.downloading")}></progress>
    {/if}
  {/if}
  {#if problem}
    <p role="alert" class="problem">{$t(`addons.manager.${problem}`)}</p>
  {:else if addon.updateFailed}
    <p role="status" class="problem">{$t("addons.manager.checkFailed")}</p>
  {:else if addon.updateChecked && !addon.update && !busy}
    <p role="status" class="hint">{$t("addons.manager.current")}</p>
  {/if}

  {#if addon.approval}
    {#key addon.approval.token}
      <AddonApproval id={addon.id} approval={addon.approval} {refresh} />
    {/key}
  {/if}

  {#snippet actions()}
    {#if !addon.approval}
    {#if addon.activity === "downloading" || addon.activity === "importing"}
      <button class="control" disabled={pending} onclick={() => void run(cancelImporter)}>{$t("library.cancel")}</button>
      {#if addon.activity === "importing" && addon.status === "enabled"}
        <button class="control" disabled={pending} onclick={() => void run(id => setAddonEnabled(id, false))}>{$t("addons.manager.disable")}</button>
        <button class="danger" disabled={pending} onclick={() => confirming = true}>{$t("addons.manager.remove")}</button>
      {/if}
    {:else}
      {#if addon.installed}
        <button class="danger" disabled={busy || pending} onclick={() => confirming = true}>{$t("addons.manager.remove")}</button>
      {/if}
      {#if addon.status !== "unsupported-os"}
        {#if addon.canCheckUpdate && addon.installed}
          <button class="control" disabled={busy || pending} onclick={() => void run(checkImporterUpdate)}>{$t("addons.manager.check")}</button>
        {/if}
        {#if addon.update}
          <button class="primary" disabled={busy || pending} onclick={() => void run(updateImporter)}>{$t("addons.manager.update")}</button>
        {:else if (addon.status === "available" || addon.status === "needs-reinstall") && addon.release}
          <button class="primary" disabled={busy || pending} onclick={() => void run(installImporter)}>{$t(addon.status === "needs-reinstall" ? "addons.manager.reinstall" : "addons.manager.install")}</button>
        {/if}
        {#if addon.status === "enabled" || addon.status === "disabled"}
          <button class="control" disabled={busy || pending} onclick={() => void run(id => setAddonEnabled(id, addon.status === "disabled"))}>
            {$t(addon.status === "enabled" ? "addons.manager.disable" : "addons.manager.enable")}
          </button>
        {/if}
      {/if}
    {/if}
    {/if}
  {/snippet}
</Dialog>

{#if confirming}
  <ConfirmDialog title={$t("addons.manager.confirmTitle", { name })} detail={$t("addons.manager.confirmDetail")}
    confirmLabel={$t("addons.manager.remove")} onCancel={() => confirming = false}
    onConfirm={() => { confirming = false; void run(removeAddon, "removeFailed"); }} />
{/if}

<style>
  .direct { display: flex; align-items: center; gap: 8px; }
  progress { width: 100%; accent-color: var(--blueprint); }
</style>
