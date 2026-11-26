<script lang="ts">
  import { shapePath } from "../shape/geometry";
  import type { Mark } from "./wasmPreview";
  let { marks }: { marks: Mark[] } = $props();
</script>
<svg aria-hidden="true" class="wasm-ghost">
  {#each marks as mark}
    <path d={shapePath(mark.geometry)} transform={`translate(${mark.x} ${mark.y}) rotate(${mark.rotation}) scale(${mark.scale})`} />
  {/each}
</svg>
<style>
  .wasm-ghost { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; z-index: 2147483646; pointer-events: none; }
  path { fill: none; stroke: var(--paper-select); stroke-width: 1.5; opacity: 0.75; }
</style>
