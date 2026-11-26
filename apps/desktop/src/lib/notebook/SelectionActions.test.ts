import { describe, expect, it } from "vitest";
import { render } from "svelte/server";
import SelectionActions from "./SelectionActions.svelte";

const base = { subject: "image" as const, left: 0, top: 0, ready: true };
const noop = () => {};

describe("SelectionActions", () => {
  it("keeps copy, duplicate and remove in the bar and the rest behind More", () => {
    const html = render(SelectionActions, {
      props: { ...base, onCopy: noop, onDuplicate: noop, onDelete: noop, onCut: noop, onMove: noop, onArrange: noop },
    }).body;
    expect(html).toContain('aria-label="Copy selected image"');
    expect(html).toContain('aria-label="Duplicate selected image"');
    expect(html).toContain('aria-label="More actions"');
    expect(html).not.toContain("Cut");
    expect(html).not.toContain("Send to back");
  });

  it("offers both cap choices with accessible labels for eligible shapes", () => {
    const html = render(SelectionActions, {
      props: { ...base, subject: "shape", shapeCaps: { endCap: "arrow", onChange: noop } },
    }).body;
    expect(html).toContain('aria-label="Start cap"');
    expect(html).toContain('aria-label="End cap"');
    expect(html).toContain('value="arrow" selected');
    expect(render(SelectionActions, { props: base }).body).not.toContain("Start cap");
  });

  it("offers Align and distribute only with two or more things selected", () => {
    const items = { ...base, subject: "items" as const, onCopy: noop };
    const align = (count: number) => ({ count, onAlign: noop, onDistribute: noop });
    expect(render(SelectionActions, { props: { ...items, align: align(2) } }).body)
      .toContain('aria-label="Align and distribute"');
    expect(render(SelectionActions, { props: { ...items, align: align(1) } }).body)
      .not.toContain("Align and distribute");
    expect(render(SelectionActions, { props: items }).body).not.toContain("Align and distribute");
  });

  it("shows Paste in the bar when a long press opened it", () => {
    const html = render(SelectionActions, { props: { ...base, onCopy: noop, onPaste: noop } }).body;
    expect(html).toContain('aria-label="Paste here"');
    expect(html).not.toContain('aria-label="More actions"');
  });
});
