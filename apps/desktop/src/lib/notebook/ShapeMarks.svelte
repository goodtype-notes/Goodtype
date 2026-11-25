<script lang="ts">
  import { shapeDashArray, shapePath, type shapeCaps } from "../shape/geometry";
  import type { ShapeStyle } from "../model";

  let { marks, style, closed }: {
    marks: ReturnType<typeof shapeCaps>;
    style: ShapeStyle;
    closed: boolean;
  } = $props();
</script>

<g opacity={style.opacity} pointer-events="none" stroke-linecap="round" stroke-linejoin="round">
  {#if marks.shaft}
    <path d={shapePath(marks.shaft)} fill={closed ? (style.fillColor ?? "none") : "none"}
      stroke={style.strokeColor} stroke-width={style.strokeWidthPt}
      stroke-dasharray={shapeDashArray(style)?.join(" ")} stroke-dashoffset={(style.patternOffsetPt ?? 0) + marks.startTrim || undefined} />
  {/if}
  {#each marks.caps as cap (cap.at)}
    {@const filled = cap.kind === "arrow" || cap.kind === "dot"}
    <path d={cap.path} fill={filled ? style.strokeColor : "none"}
      stroke={filled ? "none" : style.strokeColor} stroke-width={style.strokeWidthPt} />
  {/each}
</g>
