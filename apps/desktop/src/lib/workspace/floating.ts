/**
 * Keep a menu that hangs off a trigger inside the window.
 *
 * Every popout used to be `position: absolute` under its trigger, so it inherited the clipping of
 * whatever scrolled around it: a tile menu on the leftmost notebook was cut off by the shelf's
 * scroll box, the preset menu by the side editor, a selection menu by the bottom of the window.
 * This places the menu with `position: fixed` against the trigger's box, which escapes those
 * scroll boxes, opens it below the trigger (or above when there is more room there), aligns it to
 * the trigger's end or start edge, and clamps it 8px inside the window. A menu taller than the
 * room it gets scrolls instead of running off screen.
 */
export type FloatOptions = {
  /** What the menu hangs off. Defaults to the menu's parent element. */
  anchor?: HTMLElement | null;
  /** Which edges line up: "end" puts the menu's right edge on the trigger's right edge. */
  align?: "start" | "end";
  /** Space between trigger and menu, in CSS pixels. */
  gap?: number;
};

const MARGIN = 8;

/// The box a fixed element is laid out against. Normally the window; an ancestor with a transform,
/// filter or `will-change: transform` captures fixed descendants and becomes it instead.
function containingBlock(node: HTMLElement): DOMRect | null {
  for (let element = node.parentElement; element; element = element.parentElement) {
    const style = getComputedStyle(element);
    if (style.transform !== "none" || style.filter !== "none" || style.perspective !== "none"
      || /transform|filter/.test(style.willChange) || /paint|layout|strict|content/.test(style.contain)) {
      return element.getBoundingClientRect();
    }
  }
  return null;
}

export function placeFloating(node: HTMLElement, anchor: DOMRect, options: FloatOptions, view = { width: innerWidth, height: innerHeight }) {
  const gap = options.gap ?? 6;
  node.style.position = "fixed";
  node.style.right = "auto";
  node.style.bottom = "auto";
  node.style.maxHeight = "";
  const width = node.offsetWidth;
  let height = node.offsetHeight;
  const below = view.height - MARGIN - (anchor.bottom + gap);
  const above = anchor.top - gap - MARGIN;
  const openBelow = height <= below || below >= above;
  const room = Math.max(0, openBelow ? below : above);
  if (height > room) {
    node.style.maxHeight = `${room}px`;
    node.style.overflowY = "auto";
    height = room;
  }
  const top = openBelow ? anchor.bottom + gap : anchor.top - gap - height;
  const preferred = options.align === "start" ? anchor.left : anchor.right - width;
  const left = Math.max(MARGIN, Math.min(preferred, view.width - width - MARGIN));
  const block = containingBlock(node);
  node.style.left = `${left - (block?.left ?? 0)}px`;
  node.style.top = `${top - (block?.top ?? 0)}px`;
  node.dataset.side = openBelow ? "below" : "above";
}

/// `null` leaves the menu where its own CSS puts it, for callers that place it themselves.
export function floatInView(node: HTMLElement, options: FloatOptions | null = {}) {
  if (options === null) return {};
  let settings = options;
  let frame = 0;
  const place = () => {
    frame = 0;
    const anchor = settings.anchor ?? node.parentElement;
    if (anchor?.isConnected) placeFloating(node, anchor.getBoundingClientRect(), settings);
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(place); };
  place();
  const observer = new ResizeObserver(schedule);
  observer.observe(node);
  window.addEventListener("resize", schedule);
  // Capture: any scroll container moving the trigger, not just the window.
  window.addEventListener("scroll", schedule, true);
  return {
    update(next: FloatOptions | null = {}) { if (next) { settings = next; schedule(); } },
    destroy() {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
    },
  };
}
