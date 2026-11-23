import { describe, expect, it } from "vitest";
import { createNavigation, type NavigationEnvironment } from "../../notebook/viewportNavigation.svelte";
import { DEFAULT_SETTINGS } from "../../settings";
import { searchShapes, shapeLibraries, type ContentPack, type ShapeChoice } from "./content";
import {
  RECENT_LIMIT, createTapTracker, dragStarts, dropPoint, loadRecent, nextStamp, recentRows, rememberShape, saveRecent, shapeKey,
} from "./shapePanel";

const item = (id: string, label: string, search: string) => ({ id, label, search, widthPt: 48, heightPt: 24, objects: [] });
const pack: ContentPack = {
  id: "engineering-shapes",
  strings: {
    en: { circuits: "Circuits", logic: "Logic", resistor: "Resistor", "resistor-search": "IEC ohm passive", and: "AND gate", "and-search": "conjunction" },
    de: { circuits: "Schaltungen", logic: "Logik", resistor: "Widerstand", "resistor-search": "IEC Ohm passiv", and: "UND-Gatter", "and-search": "Konjunktion" },
  },
  shapes: [
    { id: "circuits", label: "circuits", items: [item("resistor", "resistor", "resistor-search")] },
    { id: "logic", label: "logic", items: [item("and", "and", "and-search")] },
  ],
  templates: [], palettes: [], themes: [], typstPackages: [],
};
const choice = (itemId: string, libraryId = "circuits"): ShapeChoice => ({ addonId: "engineering-shapes", libraryId, itemId });

describe("shapes panel", () => {
  it("keeps the most recent placement first, without duplicates, capped at eight", () => {
    let recent: string[] = [];
    for (let index = 0; index < 10; index += 1) recent = rememberShape(recent, choice(`s${index}`));
    expect(recent).toHaveLength(RECENT_LIMIT);
    expect(recent[0]).toBe(shapeKey(choice("s9")));
    expect(recent).not.toContain(shapeKey(choice("s1")));
    recent = rememberShape(recent, choice("s5"));
    expect(recent[0]).toBe(shapeKey(choice("s5")));
    expect(recent.filter(key => key === shapeKey(choice("s5")))).toHaveLength(1);
    expect(recent).toHaveLength(RECENT_LIMIT);
  });

  it("resolves recent keys against installed items and survives broken storage", () => {
    const libraries = shapeLibraries([pack], "en");
    const recent = [shapeKey(choice("and", "logic")), "removed-pack/x/y", shapeKey(choice("resistor"))];
    expect(recentRows(libraries, recent).map(row => row.label)).toEqual(["AND gate", "Resistor"]);

    const values = new Map<string, string>();
    const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => void values.set(key, value) };
    saveRecent(recent, storage);
    expect(loadRecent(storage)).toEqual(recent);
    expect(loadRecent({ getItem: () => "{not json" })).toEqual([]);
    expect(loadRecent({ getItem: () => { throw new Error("denied"); } })).toEqual([]);
    expect(() => saveRecent(recent, { setItem: () => { throw new Error("full"); } })).not.toThrow();
  });

  it("searches labels and search strings in the current language only", () => {
    const german = shapeLibraries([pack], "de");
    expect(searchShapes(german, "", "ohm", "de").map(row => row.label)).toEqual(["Widerstand"]);
    expect(searchShapes(german, "", "konjunktion", "de").map(row => row.label)).toEqual(["UND-Gatter"]);
    expect(searchShapes(german, "", "conjunction", "de")).toEqual([]);
    const english = shapeLibraries([pack], "en");
    expect(searchShapes(english, "", "gate", "en").map(row => row.label)).toEqual(["AND gate"]);
    expect(searchShapes(english, "", "Widerstand", "en")).toEqual([]);
  });

  it("enters stamp mode on a tile and leaves it on re-tap, Esc, a tool change or closing", () => {
    const resistor = choice("resistor");
    const gate = choice("and", "logic");
    let stamp = nextStamp(null, { type: "tile", choice: resistor });
    expect(stamp).toEqual(resistor);
    stamp = nextStamp(stamp, { type: "tile", choice: gate });
    expect(stamp).toEqual(gate);
    expect(nextStamp(stamp, { type: "tile", choice: { ...gate } })).toBeNull();
    expect(nextStamp(stamp, { type: "escape" })).toBeNull();
    expect(nextStamp(stamp, { type: "tool" })).toBeNull();
    expect(nextStamp(stamp, { type: "close" })).toBeNull();
  });

  it("places only for a short, still press with one pointer", () => {
    const taps = createTapTracker();
    taps.down(1, 100, 100, 0, "p1");
    expect(taps.up(1, 103, 102, 120)).toEqual({ x: 100, y: 100, pageId: "p1" });

    taps.down(1, 100, 100, 0, "p1");
    taps.move(1, 140, 100);
    expect(taps.up(1, 140, 100, 120)).toBeNull();

    taps.down(1, 100, 100, 0, "p1");
    expect(taps.up(1, 100, 100, 900)).toBeNull();

    taps.down(1, 100, 100, 0, "p1");
    taps.down(2, 200, 100, 10, "p1");
    expect(taps.up(1, 100, 100, 50)).toBeNull();

    taps.down(1, 100, 100, 0, "p1");
    expect(taps.up(1, 100, 100, 50, true)).toBeNull();
  });

  it("drags a tile with mouse or pen after a few pixels and with a finger only sideways or after a hold", () => {
    expect(dragStarts("mouse", 2, 2, 10)).toBe(false);
    expect(dragStarts("pen", 0, 9, 10)).toBe(true);
    expect(dragStarts("touch", 0, 30, 100)).toBe(false);
    expect(dragStarts("touch", -30, 6, 100)).toBe(true);
    expect(dragStarts("touch", 0, 2, 400)).toBe(true);
  });

  it("converts a drop point through the coordinate authority and refuses points off the page", () => {
    const frame = { left: 100, top: 200, right: 500, bottom: 800 };
    const env = { settings: DEFAULT_SETTINGS, zoom: 2, collectMetrics: false,
      pageFrame: { getBoundingClientRect: () => frame },
      pageViewport: { scrollLeft: 10, scrollTop: 20 } } as NavigationEnvironment;
    const navigation = createNavigation(env);
    expect(dropPoint(navigation, frame, 180, 300)).toEqual({ x: 40, y: 50 });
    expect(dropPoint(navigation, frame, 90, 300)).toBeNull();
    expect(dropPoint(navigation, undefined, 180, 300)).toBeNull();
    env.zoom = 4;
    expect(dropPoint(navigation, frame, 180, 300)).toEqual({ x: 20, y: 25 });
  });
});
