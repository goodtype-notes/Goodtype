<script lang="ts">
  import { pagePorts } from "./addonPorts";
  import type { Point } from "../geometry/coordinates";
  import type { PageObject } from "../model";
  import type { AddonPoint, AddonTool } from "../ipc/addons";

  let { tool, label, zoom, objects, pointAt, onApply }: {
    tool: AddonTool; label: string; zoom: number; objects: PageObject[];
    pointAt: (x: number, y: number) => Point | null; onApply: (points: AddonPoint[]) => void;
  } = $props();

  type Port = Point & { attach: NonNullable<AddonPoint["attach"]> };
  const ports = $derived(tool.snap !== "ports" ? [] : objects.flatMap((object) =>
    pagePorts(object).map((point, port): Port => ({ ...point, attach: { objectId: object.id, port } }))));
  let start = $state<AddonPoint | null>(null);
  let pointer = $state<AddonPoint | null>(null);
  let nearby = $state<Port[]>([]);
  let captured = $state<number | null>(null);

  function sample(event: PointerEvent) {
    const point = pointAt(event.clientX, event.clientY);
    if (!point) return null;
    const distance = (p: Point) => Math.hypot(p.x - point.x, p.y - point.y) * zoom;
    nearby = ports.filter((port) => distance(port) <= 32);
    let match: Port | undefined;
    let limit = 12;
    for (const port of nearby) {
      const offset = distance(port);
      if (offset <= limit && (!match || offset < limit)) {
        match = port;
        limit = offset;
      }
    }
    return match ?? { ...point, attach: null };
  }

  function down(event: PointerEvent) {
    if (!event.isPrimary || event.button !== 0 || captured !== null) return;
    const point = sample(event);
    if (!point) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    captured = event.pointerId;
    start = pointer = point;
  }

  function finish(event: PointerEvent) {
    if (captured !== event.pointerId) return;
    const end = sample(event);
    const first = start;
    captured = null;
    start = null;
    if (!first || !end) return;
    onApply(tool.kind === "place" ? [end] : [first, end]);
  }

  function clear() {
    captured = null;
    start = pointer = null;
    nearby = [];
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="surface" role="region" aria-label={label}
  onpointerdown={down} onpointerup={finish} onpointercancel={clear}
  onlostpointercapture={() => { if (captured !== null) clear(); }}
  onpointerleave={() => { if (captured === null) clear(); }}
  onpointermove={(event) => { if (captured === null || captured === event.pointerId) pointer = sample(event); }}>
  <svg aria-hidden="true" width="100%" height="100%">
    {#if pointer && tool.kind === "place"}
      <rect x={pointer.x - 8} y={pointer.y - 8} width="16" height="16" />
    {:else if start && pointer && tool.kind === "connect"}
      <line x1={start.x} y1={start.y} x2={pointer.x} y2={pointer.y} />
    {:else if start && pointer && tool.kind === "frame"}
      <rect x={Math.min(start.x, pointer.x)} y={Math.min(start.y, pointer.y)}
        width={Math.abs(start.x - pointer.x)} height={Math.abs(start.y - pointer.y)} />
    {/if}
    {#each nearby as port}
      <circle class:snapped={pointer?.attach?.objectId === port.attach.objectId && pointer?.attach?.port === port.attach.port}
        cx={port.x} cy={port.y} r={4 / zoom} />
    {/each}
  </svg>
</div>

<style>
  .surface { position: absolute; inset: 0; z-index: 2147483647; cursor: crosshair; touch-action: none; }
  svg { pointer-events: none; }
  line, rect, circle { fill: none; stroke: var(--paper-select); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
  circle { fill: var(--paper); }
  circle.snapped { fill: var(--paper-select); }
</style>
