<script lang="ts">
  import { shapeDashArray, shapePath } from "../../shape/geometry";
  import type { PackShape } from "./content";
  import { previewMargin } from "./shapePanel";

  let { item, color = "#1e232b", width = 1.6 }: {
    item: PackShape;
    color?: string;
    width?: number;
  } = $props();
  const margin = $derived(previewMargin(item, width));
</script>

<svg aria-hidden="true" viewBox={`${-margin} ${-margin} ${item.widthPt + 2 * margin} ${item.heightPt + 2 * margin}`}>
  {#each item.objects as object}
    {#if object.type === "shape"}
      {@const style = {
        ...object.style,
        strokeColor: object.style.strokeColor ?? color,
        strokeWidthPt: object.style.strokeWidthPt ?? width,
        fillColor: object.style.fillColor === "pen" ? color : object.style.fillColor ?? null,
        opacity: object.style.opacity ?? 1,
      }}
      <path d={shapePath(object.geometry)} transform={`translate(${object.x} ${object.y})`}
        stroke={style.strokeColor} stroke-width={style.strokeWidthPt} fill={style.fillColor ?? "none"}
        opacity={style.opacity} stroke-dasharray={shapeDashArray(style)?.join(" ")}
        stroke-linecap="round" stroke-linejoin="round" />
    {/if}
  {/each}
</svg>

<style>
  svg { display: block; width: 100%; height: 100%; }
</style>
