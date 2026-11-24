import { tick } from "svelte";
import { dismissable } from "../workspace/dismiss";
import type { PaletteDock } from "./palette";

export function placePalettePopup(node: HTMLElement, options: {
  anchor: HTMLElement; dock: PaletteDock; onDismiss: () => void; focus?: string;
} | undefined) {
  if (!options) return;
  const { anchor, dock, onDismiss, focus } = options;
  const palette = anchor.closest<HTMLElement>(".instrument-palette")!;
  const workspace = anchor.closest<HTMLElement>(".workspace-chrome") ?? palette.parentElement!;
  // Include the rail in the dismissal boundary: pressing the opener again must close, not reopen.
  const dismiss = dismissable(palette, onDismiss);
  const position = () => {
    const chip = anchor.getBoundingClientRect();
    const rail = palette.getBoundingClientRect();
    const bounds = workspace.getBoundingClientRect();
    const box = node.getBoundingClientRect();
    const x = dock === "left" ? rail.right + 10 : dock === "right" ? rail.left - box.width - 10 : chip.left + chip.width / 2 - box.width / 2;
    const y = dock === "top" ? rail.bottom + 10 : dock === "bottom" ? rail.top - box.height - 10 : chip.top + chip.height / 2 - box.height / 2;
    const left = Math.max(bounds.left + 8, Math.min(x, bounds.right - box.width - 8));
    const top = Math.max(bounds.top + 8, Math.min(y, bounds.bottom - box.height - 8));
    node.style.left = `${left}px`;
    node.style.top = `${top}px`;
    node.style.setProperty("--arrow", `${dock === "left" || dock === "right"
      ? chip.top + chip.height / 2 - top : chip.left + chip.width / 2 - left}px`);
  };
  position();
  const observer = new ResizeObserver(position);
  observer.observe(node);
  observer.observe(workspace);
  observer.observe(palette);
  palette.addEventListener("scroll", position, true);
  let mounted = true;
  void tick().then(() => {
    if (mounted) (focus ? node.querySelector<HTMLElement>(focus) : node)?.focus();
  });
  return { destroy() { mounted = false; observer.disconnect(); palette.removeEventListener("scroll", position, true); dismiss.destroy(); } };
}
