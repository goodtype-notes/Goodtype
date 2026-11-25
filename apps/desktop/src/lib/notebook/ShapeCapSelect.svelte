<script lang="ts">
  import { t } from "../i18n";
  import type { ShapeCap } from "../model";
  import { SHAPE_CAPS } from "../shape/geometry";

  let { end, value = "none", onChange }: {
    end: "start" | "end";
    value?: ShapeCap;
    onChange: (cap: ShapeCap) => void;
  } = $props();
  const label = $derived($t(end === "start" ? "workspace.shapeStartCap" : "workspace.shapeEndCap"));
</script>

<label>
  <span>{label}</span>
  <select {value} aria-label={label} onchange={(event) => onChange(event.currentTarget.value as ShapeCap)}>
    {#each SHAPE_CAPS as cap (cap)}
      <option value={cap}>{$t(`workspace.shapeCap.${cap}`)}</option>
    {/each}
  </select>
</label>

<style>
  label {
    display: grid;
    gap: 3px;
    color: var(--muted);
    font-size: var(--text-sm);
  }
  select {
    max-width: 100%;
    min-height: var(--control-touch);
    color: var(--text);
    background: var(--panel, var(--paper));
    border: 1px solid var(--edge);
    border-radius: var(--radius);
    font: inherit;
    padding: 0 4px;
  }
  select:focus-visible { outline: 2px solid var(--blueprint-light); }
</style>
