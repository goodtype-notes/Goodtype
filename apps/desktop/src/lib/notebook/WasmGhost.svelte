<script lang="ts">
  import { shapePath } from "../shape/geometry";
  import { t } from "../i18n";
  import type { Mark } from "./wasmPreview";
  let { marks, owner }: { marks: Mark[]; owner: string } = $props();
</script>
<svg role="img" aria-label={$t("addons.previewOwner", { name: owner })} class="wasm-ghost">
  {#each marks as mark}
    <g transform={`translate(${mark.x} ${mark.y}) rotate(${mark.rotation}) scale(${mark.scale})`} stroke={mark.style.color} stroke-width={mark.style.width}>
      {#if mark.geometry.kind === "label"}
        <text fill={mark.style.color} stroke="none" font-size="12">{mark.geometry.text}</text>
      {:else if mark.geometry.kind === "ring"}
        <circle r={mark.geometry.radius} />
      {:else if mark.geometry.kind === "polyline"}
        <polyline points={mark.geometry.points.map(p => `${p.x},${p.y}`).join(" ")} />
      {:else if mark.geometry.kind === "bezier"}
        {@const p = mark.geometry.points}
        <path d={`M${p[0].x} ${p[0].y} C${p[1].x} ${p[1].y} ${p[2].x} ${p[2].y} ${p[3].x} ${p[3].y}`} />
      {:else}
        <path d={shapePath(mark.geometry)} />
      {/if}
    </g>
  {/each}
</svg>
<style>
  .wasm-ghost { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; z-index: 2147483646; pointer-events: none; fill: none; opacity: 0.75; }
</style>
