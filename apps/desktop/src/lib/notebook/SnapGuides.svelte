<script lang="ts">
  import type { SnapGuide } from "../geometry/alignSnap";

  /**
   * The lines a dragged object has just met on other objects. Page points inside the page's own
   * transform, drawn one screen pixel thick whatever the zoom. Purely visual: they carry no input,
   * no accessible name and nothing that is saved.
   */
  let { guides, zoom }: { guides: readonly SnapGuide[]; zoom: number } = $props();
</script>

<svg class="snap-guides" aria-hidden="true" style:--zoom={zoom}>
  {#each guides as guide (`${guide.axis}:${guide.at}`)}
    {#if guide.axis === "x"}
      <line x1={guide.at} x2={guide.at} y1={guide.from} y2={guide.to} />
    {:else}
      <line y1={guide.at} y2={guide.at} x1={guide.from} x2={guide.to} />
    {/if}
  {/each}
</svg>

<style>
  .snap-guides { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; z-index: 2147483646; pointer-events: none; }
  line { stroke: var(--paper-select); stroke-width: calc(1px / var(--zoom)); }
</style>
