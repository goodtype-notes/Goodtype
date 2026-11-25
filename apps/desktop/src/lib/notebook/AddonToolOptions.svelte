<script lang="ts">
  import { locale } from "../i18n";
  import { contributionText, type AddonContribution, type AddonField, type AddonValue } from "../ipc/addons";
  import AddonFieldControl from "../workspace/addons/AddonField.svelte";
  import { toolFieldPresentation } from "./addonToolSettings";

  let { addon, fields, rail = false, disabled, onChange }: {
    addon: AddonContribution;
    fields: AddonField[];
    rail?: boolean;
    disabled: boolean;
    onChange: (id: string, value: AddonValue) => void;
  } = $props();
</script>

{#each fields as field (field.id)}
  {@const label = contributionText(addon, field.label, $locale)}
  {@const value = addon.values[field.id] ?? field.default}
  {@const mode = toolFieldPresentation(addon, field, $locale)}
  {#if field.type === "choice"}
    <div class="choice" class:rail>
      {#if !rail}<span class="label">{label}</span>{/if}
      <div class="options" class:list={mode === "list"} role="group" aria-label={label}>
        {#each field.choices as option (option.value)}
          {@const name = contributionText(addon, option.label, $locale)}
          <button type="button" class:icon={mode === "icons"} {disabled}
            aria-pressed={value === option.value} aria-label={name} title={name}
            onclick={() => onChange(field.id, option.value)}>
            {#if mode === "icons"}
              <svg viewBox="0 0 24 24" aria-hidden="true">
                {#each option.icon?.paths ?? [] as path}<path d={path} />{/each}
              </svg>
            {:else}{name}{/if}
          </button>
        {/each}
      </div>
    </div>
  {:else}
    <AddonFieldControl {addon} {field} {value} {disabled} onChange={(next) => onChange(field.id, next)} />
  {/if}
{/each}

<style>
  .choice { display: grid; gap: 4px; min-width: 0; }
  .label { color: var(--muted); font-size: var(--text-sm); font-weight: 600; }
  .options { display: flex; padding: 3px; gap: 2px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); }
  .options.list { flex-direction: column; }
  button {
    flex: 1; min-height: var(--control-dense); padding: 3px 8px; border: 0;
    border-radius: var(--radius); background: transparent; color: var(--muted);
    font: inherit; font-size: var(--text-md); cursor: pointer;
    transition: background var(--fast) ease, color var(--fast) ease;
  }
  .rail button { white-space: nowrap; }
  button.icon { display: grid; min-width: var(--control-dense); place-items: center; }
  .list button { text-align: start; }
  button:hover:not(:disabled):not([aria-pressed="true"]) { background: var(--wash); color: var(--text); }
  button:active:not(:disabled) { background: var(--wash-strong); }
  button[aria-pressed="true"] { background: var(--accent-fill); box-shadow: inset 0 0 0 1px var(--accent-edge); color: var(--text); font-weight: 600; }
  button:focus-visible { position: relative; outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  button:disabled { opacity: var(--disabled); cursor: default; }
  svg { width: var(--icon-dense); height: var(--icon-dense); fill: none; stroke: currentColor;
    stroke-width: var(--stroke-dense); stroke-linecap: round; stroke-linejoin: round; }
  @media (prefers-reduced-motion: reduce) { button { transition: none; } }
</style>
