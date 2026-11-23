<script lang="ts">
  import { locale } from "../../i18n";
  import { contributionText, type AddonContribution, type AddonField, type AddonValue } from "../../ipc/addons";

  let { addon, field, value, onChange, disabled = false, invalid = false }: {
    addon: AddonContribution;
    field: AddonField;
    value: AddonValue | undefined;
    onChange: (value: AddonValue) => void;
    disabled?: boolean;
    invalid?: boolean;
  } = $props();

  const id = $props.id();
  const label = $derived(contributionText(addon, field.label, $locale));
  const hint = $derived(field.hint ? contributionText(addon, field.hint, $locale) : "");
</script>

<div class="field">
  {#if field.type === "toggle"}
    <button type="button" class="switch-row" role="switch" aria-checked={value === true}
      aria-describedby={hint ? id : undefined} {disabled} onclick={() => onChange(value !== true)}>
      {label}<span></span>
    </button>
  {:else if field.type === "choice"}
    <fieldset {disabled} aria-describedby={hint ? id : undefined}>
      <legend>{label}</legend>
      <div class="segments">
        {#each field.choices as choice (choice.value)}
          <label><input type="radio" name={id} checked={choice.value === value} onchange={() => onChange(choice.value)} />
            <span>{contributionText(addon, choice.label, $locale)}</span>
          </label>
        {/each}
      </div>
    </fieldset>
  {:else}
    <label for={id + "-input"}>{label}</label>
    {#if field.type === "text"}
      <input id={id + "-input"} type="text" value={String(value ?? "")} {disabled}
        aria-invalid={invalid} aria-describedby={hint ? id : undefined} oninput={event => onChange(event.currentTarget.value)} />
    {:else}
      <input id={id + "-input"} type="number" value={value} min={field.min} max={field.max} step={field.step}
        {disabled} aria-invalid={invalid} aria-describedby={hint ? id : undefined}
        oninput={event => onChange(event.currentTarget.valueAsNumber)} />
    {/if}
  {/if}
  {#if hint}<p id={id}>{hint}</p>{/if}
</div>

<style>
  .field { display: grid; gap: 6px; min-width: 0; padding-block: 6px; }
  .field p { margin: 0; color: var(--muted); font-size: var(--text-sm); }
  fieldset { padding: 0; border: 0; margin: 0; min-width: 0; }
  legend { padding: 0; margin-block-end: 6px; }
  .segments { display: flex; flex-wrap: wrap; gap: 4px; }
  .segments label { position: relative; }
  .segments input { position: absolute; width: 1px; height: 1px; opacity: 0; }
  .segments span { display: grid; place-items: center; min-height: var(--control); padding-inline: 10px; border-radius: var(--radius); background: var(--charcoal); }
  .segments input:checked + span { background: var(--wash); color: var(--text); }
  .segments input:focus-visible + span { outline: 2px solid var(--blueprint); outline-offset: 2px; }
  .switch-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; text-align: start; min-height: var(--control); color: inherit; font: inherit; border: 0; background: transparent; cursor: pointer; }
  .switch-row span { box-sizing: border-box; width: 34px; height: 20px; padding: 3px; border-radius: var(--radius-pill); background: var(--track); flex: none; }
  .switch-row span::before { content: ""; display: block; width: 14px; height: 14px; border-radius: 50%; background: var(--muted); }
  .switch-row[aria-checked="true"] span { background: var(--blueprint); }
  .switch-row[aria-checked="true"] span::before { background: var(--text); transform: translateX(14px); }
  .field > input { box-sizing: border-box; width: 100%; padding: 6px 8px; min-height: var(--control); border: 1px solid var(--edge-soft); border-radius: var(--radius); background: var(--charcoal); color: var(--text); font: inherit; }
  .field > input[aria-invalid="true"] { border-color: var(--oxide); }
</style>
