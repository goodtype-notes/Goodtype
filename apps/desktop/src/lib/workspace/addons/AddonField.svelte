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
  .field { display: grid; gap: 4px; min-width: 0; padding-block: 8px; }
  .field > label, legend { padding: 0; color: var(--muted); font-size: var(--text-sm); font-weight: 600; line-height: 1.4; }
  .field p { margin: 0; color: var(--quiet); font-size: var(--text-sm); line-height: 1.4; }
  fieldset { min-width: 0; margin: 0; padding: 0; border: 0; }
  legend { margin-block-end: 4px; }
  .segments { display: flex; flex-wrap: wrap; width: fit-content; max-width: 100%; padding: 3px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); gap: 3px; }
  .segments label { position: relative; }
  .segments input { position: absolute; width: 1px; height: 1px; opacity: 0; }
  .segments span { display: grid; place-items: center; min-height: calc(var(--control) - 8px); padding-inline: 12px; border-radius: var(--radius); color: var(--muted); font-size: var(--text-md); cursor: pointer; transition: background var(--fast) ease, color var(--fast) ease; }
  .segments label:hover span { background: var(--wash); color: var(--text); }
  .segments input:checked + span { background: var(--accent-fill); box-shadow: inset 0 0 0 1px var(--accent-edge); color: var(--text); }
  .segments input:focus-visible + span { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  fieldset:disabled .segments { opacity: var(--disabled); }
  fieldset:disabled .segments span { cursor: default; }
  fieldset:disabled .segments label:hover span { background: transparent; color: var(--muted); }
  .switch-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: var(--control); padding: 0; border: 0; background: transparent; color: var(--text); font: inherit; font-size: var(--text-md); text-align: start; cursor: pointer; }
  .switch-row:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; border-radius: var(--radius); }
  .switch-row:disabled { opacity: var(--disabled); cursor: default; }
  .switch-row span { box-sizing: border-box; width: 34px; height: 20px; padding: 3px; border-radius: var(--radius-pill); background: var(--track); flex: none; transition: background var(--fast) ease; }
  .switch-row span::before { content: ""; display: block; width: 14px; height: 14px; border-radius: 50%; background: var(--muted); transition: transform var(--fast) ease, background var(--fast) ease; }
  .switch-row[aria-checked="true"] span { background: var(--blueprint); }
  .switch-row[aria-checked="true"] span::before { background: var(--text); transform: translateX(14px); }
  .field > input { box-sizing: border-box; width: 100%; min-height: var(--control); padding: 0 12px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); color: var(--text); font: inherit; font-size: var(--text-md); transition: border-color var(--fast) ease; }
  .field > input:hover:not(:disabled) { border-color: var(--edge-strong); }
  .field > input:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  .field > input:disabled { opacity: var(--disabled); }
  .field > input[aria-invalid="true"] { border-color: var(--oxide); }
  @media (pointer: coarse) {
    .field > input, .switch-row { min-height: var(--control-touch); }
    .segments span { min-height: calc(var(--control-touch) - 8px); }
  }
  @media (prefers-reduced-motion: reduce) {
    .segments span, .switch-row span, .switch-row span::before, .field > input { transition: none; }
  }
</style>
