<script lang="ts">
  import { locale, t } from "../../i18n";
  import { contributionText, setAddonField, type AddonContribution, type AddonValue } from "../../ipc/addons";
  import AddonField from "./AddonField.svelte";
  import { matchesField } from "./formModel";

  let { addon, query = "", disabled = false }: { addon: AddonContribution; query?: string; disabled?: boolean } = $props();
  let changes = $state<Record<string, AddonValue>>({});
  let pending = $state(false);
  let error = $state(false);
  const fields = $derived(addon.settings.filter(field => matchesField(addon, field, query, $locale)));

  async function update(id: string, value: AddonValue) {
    if (pending) return;
    pending = true;
    error = false;
    try {
      await setAddonField(addon.addonId, id, value);
      changes[id] = value;
    } catch {
      error = true;
    } finally {
      pending = false;
    }
  }
</script>

{#if fields.length}
  <section data-ui="solid" aria-label={contributionText(addon, "name", $locale)}>
    <h3>{query ? contributionText(addon, "name", $locale) : $t("addons.contributions.settings")}</h3>
    {#each fields as field (field.id)}
      {@const value = changes[field.id] ?? addon.values[field.id] ?? field.default}
      <AddonField {addon} {field} {value}
        disabled={disabled || pending} onChange={(value) => void update(field.id, value)} />
    {/each}
    {#if error}<p role="alert">{$t("addons.manager.policyFailed")}</p>{/if}
  </section>
{/if}

<style>
  section { padding-block: 12px; border-bottom: 1px solid var(--edge-soft); }
  h3 { margin: 0 0 8px; font-size: var(--text-md); font-weight: 600; line-height: 1.4; }
  p { margin: 8px 0 0; color: var(--oxide-text); font-size: var(--text-sm); line-height: 1.4; }
</style>
