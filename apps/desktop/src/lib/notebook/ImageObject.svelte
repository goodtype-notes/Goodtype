<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "../i18n";
  import { keepOnPage } from "../geometry/placement";
  import { rotatedVector } from "../geometry/coordinates";
  import type { ObjectMoveSnap } from "../geometry/alignSnap";

  type Position = { x: number; y: number };
  type Gesture =
    | {
        kind: "move";
        pointerId: number;
        clientX: number;
        clientY: number;
        x: number;
        y: number;
      }
    | {
        kind: "scale";
        pointerId: number;
        clientX: number;
        clientY: number;
        scale: number;
      };

  type Props = {
    id: string;
    src: string;
    alt: string;
    x: number;
    y: number;
    widthPt: number;
    heightPt: number;
    scale: number;
    rotation?: number;
    zIndex?: number;
    readingOrder?: number;
    selected?: boolean;
    zoom?: number;
    toPageDelta?: (screenDx: number, screenDy: number) => Position;
    /** Pulls a dragged image onto other objects' edges, centres and ports; Alt bypasses it. */
    moveSnap?: ObjectMoveSnap;
    /** The sheet this image has to stay reachable on. */
    pageWidthPt: number;
    pageHeightPt: number;
    onSelect?: () => void;
    onMove?: (position: Position) => void;
    onScale?: (scale: number) => void;
  };

  let {
    id,
    src,
    alt,
    x,
    y,
    widthPt,
    heightPt,
    scale,
    rotation = 0,
    zIndex = 0,
    readingOrder = 0,
    selected = false,
    zoom = 1,
    moveSnap,
    toPageDelta = (screenDx, screenDy) => ({ x: screenDx, y: screenDy }),
    pageWidthPt,
    pageHeightPt,
    onSelect,
    onMove,
    onScale,
  }: Props = $props();

  let previewX = $state(0);
  let previewY = $state(0);
  let previewScale = $state(1);
  let gesture = $state<Gesture | null>(null);

  $effect(() => {
    if (!gesture) {
      previewX = x;
      previewY = y;
      previewScale = scale;
    }
  });

  // A drag the image never gets to end (undo, delete or a page change unmounts it) must not leave
  // its guides on the page.
  onDestroy(() => {
    if (gesture?.kind === "move") moveSnap?.end();
  });

  function beginMove(event: PointerEvent) {
    if (event.button !== 0) return;
    onSelect?.();
    event.currentTarget instanceof HTMLElement &&
      event.currentTarget.setPointerCapture(event.pointerId);
    gesture = {
      kind: "move",
      pointerId: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
      x: previewX,
      y: previewY,
    };
    moveSnap?.begin([id]);
  }

  function beginScale(event: PointerEvent) {
    if (event.button !== 0) return;
    event.stopPropagation();
    onSelect?.();
    event.currentTarget instanceof HTMLElement &&
      event.currentTarget.setPointerCapture(event.pointerId);
    gesture = {
      kind: "scale",
      pointerId: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
      scale: previewScale,
    };
  }

  function update(event: PointerEvent) {
    if (!gesture || event.pointerId !== gesture.pointerId) return;
    const delta = toPageDelta(
      event.clientX - gesture.clientX,
      event.clientY - gesture.clientY,
    );
    if (gesture.kind === "move") {
      const { x: startX, y: startY } = gesture;
      let held: Position = { x: startX, y: startY };
      // Clamped while dragging rather than on drop, so the limit is something you feel rather
      // than something that snaps the image back after you let go. The snapper measures its
      // guides at the clamped spot.
      const clamp = (moved: Position): Position => {
        held = keepOnPage(
          { x: startX + moved.x, y: startY + moved.y },
          { widthPt: widthPt * previewScale, heightPt: heightPt * previewScale },
          { widthPt: pageWidthPt, heightPt: pageHeightPt },
          undefined, rotation,
        );
        return { x: held.x - startX, y: held.y - startY };
      };
      if (moveSnap) moveSnap.adjust(delta, event.altKey, clamp);
      else clamp(delta);
      previewX = held.x;
      previewY = held.y;
    } else {
      previewScale = Math.max(0.1, gesture.scale + rotatedVector(delta, -rotation).x / widthPt);
    }
  }

  function finish(event: PointerEvent) {
    if (!gesture || event.pointerId !== gesture.pointerId) return;
    if (gesture.kind === "move") onMove?.({ x: previewX, y: previewY });
    else onScale?.(previewScale);
    moveSnap?.end();
    gesture = null;
  }

  function cancel(event: PointerEvent) {
    if (!gesture || event.pointerId !== gesture.pointerId) return;
    moveSnap?.end();
    gesture = null;
    previewX = x;
    previewY = y;
    previewScale = scale;
  }

  function keydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 10 : 1;
    const moves: Record<string, Position> = {
      ArrowLeft: { x: x - step, y },
      ArrowRight: { x: x + step, y },
      ArrowUp: { x, y: y - step },
      ArrowDown: { x, y: y + step },
    };
    if (moves[event.key]) {
      event.preventDefault();
      // Nudging is bounded like dragging is; an arrow key held down would otherwise walk the
      // image off the sheet just as effectively.
      onMove?.(
        keepOnPage(
          moves[event.key],
          { widthPt: widthPt * scale, heightPt: heightPt * scale },
          { widthPt: pageWidthPt, heightPt: pageHeightPt },
          undefined, rotation,
        ),
      );
    } else if (event.key === "+" || event.key === "=") {
      event.preventDefault();
      onScale?.(scale + 0.1);
    } else if (event.key === "-") {
      event.preventDefault();
      onScale?.(Math.max(0.1, scale - 0.1));
    }
  }
</script>

<div
  class:selected
  class="image-object"
  data-object-id={id}
  style:left={`${previewX}px`}
  style:top={`${previewY}px`}
  style:width={`${widthPt}px`}
  style:height={`${heightPt}px`}
  style:transform={`rotate(${rotation}deg) scale(${previewScale})`}
  style:--selection-scale={1 / (zoom * previewScale)}
  style:z-index={zIndex}
  aria-label={$t("workspace.readingPosition2", { "alt": alt, "value2": readingOrder + 1 })}
>
  <button
    class="move-surface"
    type="button"
    aria-pressed={selected}
    aria-label={$t("workspace.move", { "alt": alt })}
    onpointerdown={beginMove}
    onpointermove={update}
    onpointerup={finish}
    onpointercancel={cancel}
    onkeydown={keydown}
  >
    <img {src} {alt} draggable="false" />
  </button>
  {#if selected}
    <svg class="selection-frame" aria-hidden="true"><rect class="selection-box" width="100%" height="100%" /></svg>
    <button
      class="scale-handle selection-html-handle"
      type="button"
      aria-label={$t("workspace.scale", { "alt": alt })}
      onpointerdown={beginScale}
      onpointermove={update}
      onpointerup={finish}
      onpointercancel={cancel}
    ></button>
  {/if}
</div>

<style>
  .image-object {
    position: absolute;
    box-sizing: border-box;
    transform-origin: top left;
    touch-action: none;
    user-select: none;
  }

  .move-surface {
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: move;
  }

  .move-surface:focus-visible {
    outline: calc(var(--selection-focus-stroke) * var(--selection-scale)) solid var(--selection-color);
    outline-offset: 0;
  }

  img {
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .scale-handle {
    position: absolute;
    right: calc(-1 * var(--selection-hit-radius));
    bottom: calc(-1 * var(--selection-hit-radius));
    transform: scale(var(--selection-scale));
    cursor: nwse-resize;
  }
</style>
