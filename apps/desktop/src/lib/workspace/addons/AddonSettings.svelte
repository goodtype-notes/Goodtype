<script lang="ts">
  import { locale, t } from "../../i18n";
  import { contributionText, setAddonField, type AddonContribution, type AddonValue } from "../../ipc/addons";
  import AddonField from "./AddonField.svelte";
  import { matchesField } from "./formModel";

  let { addon, query = "", disabled = false, compact = false }: { addon: AddonContribution; query?: string; disabled?: boolean; compact?: boolean } = $props();
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
  <section class:compact aria-label={compact ? $t("addons.contributions.settings") : contributionText(addon, "name", $locale)}>
    {#if !compact}
      <h3>{query ? contributionText(addon, "name", $locale) : $t("addons.contributions.settings")}</h3>
    {/if}
    {#each fields as field (field.id)}
      {@const value = changes[field.id] ?? addon.values[field.id] ?? field.default}
      {#if compact && field.type === "choice"}
        <label class="choice">
          <span>{contributionText(addon, field.label, $locale)}</span>
          <select {value} disabled={disabled || pending} onchange={(event) => void update(field.id, event.currentTarget.value)}>
            {#each field.choices as choice (choice.value)}
              <option value={choice.value}>{contributionText(addon, choice.label, $locale)}</option>
            {/each}
          </select>
        </label>
      {:else}
        <AddonField {addon} {field} {value}
          disabled={disabled || pending} onChange={(value) => void update(field.id, value)} />
      {/if}
    {/each}
    {#if error}<p role="alert">{$t("addons.manager.policyFailed")}</p>{/if}
  </section>
{/if}

<style>
  section { padding-block: 12px; border-bottom: 1px solid var(--edge-soft); }
  .compact { display: flex; align-items: center; gap: 8px; color: var(--text); padding: 0; border: 0; }
  .choice { display: grid; gap: 2px; font-size: var(--text-sm); color: var(--muted); }
  .choice select { min-height: var(--control); max-width: 130px; color: var(--text); background: var(--charcoal);
    border: 1px solid var(--edge-soft); border-radius: var(--radius); font: inherit; }
  h3 { margin: 0 0 4px; font-size: var(--text-md); }
  p { color: var(--muted); }
</style>
