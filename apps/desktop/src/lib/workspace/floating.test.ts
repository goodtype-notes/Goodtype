import { expect, it } from "vitest";
import { placeFloating } from "./floating";

const view = { width: 1000, height: 700 };

function menu(width: number, height: number) {
  return { style: {} as Record<string, string>, dataset: {} as Record<string, string>, offsetWidth: width, offsetHeight: height, parentElement: null } as unknown as HTMLElement;
}

function anchor(left: number, top: number, width = 32, height = 32) {
  return { left, top, right: left + width, bottom: top + height, width, height } as DOMRect;
}

it("opens below the trigger with the end edges aligned", () => {
  const node = menu(200, 150);
  placeFloating(node, anchor(500, 100), { align: "end" }, view);
  expect(node.style.left).toBe("332px");
  expect(node.style.top).toBe("138px");
  expect(node.dataset.side).toBe("below");
});

it("stays 8px inside the left edge for a trigger near it (the leftmost library tile)", () => {
  const node = menu(200, 150);
  placeFloating(node, anchor(20, 100), { align: "end" }, view);
  expect(node.style.left).toBe("8px");
});

it("stays inside the right edge for a start-aligned menu", () => {
  const node = menu(300, 150);
  placeFloating(node, anchor(900, 100), { align: "start" }, view);
  expect(node.style.left).toBe("692px");
});

it("flips above the trigger when there is more room there", () => {
  const node = menu(200, 300);
  placeFloating(node, anchor(500, 600), { align: "end" }, view);
  expect(node.dataset.side).toBe("above");
  expect(node.style.top).toBe("294px");
});

it("scrolls instead of running off screen when neither side has room", () => {
  const node = menu(200, 900);
  placeFloating(node, anchor(500, 100), { align: "end" }, view);
  expect(node.dataset.side).toBe("below");
  expect(node.style.maxHeight).toBe("554px");
  expect(node.style.overflowY).toBe("auto");
});
