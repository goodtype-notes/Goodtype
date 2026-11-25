import { describe, expect, it } from "vitest";
import type { AddonContribution, AddonField } from "../ipc/addons";
import { DEFAULT_SETTINGS } from "../settings";
import { settingGroups } from "../workspace/settingsSchema";
import { defaultAddonStrokeSlot, fitsAddonSegments, toolFieldPresentation } from "./addonToolSettings";

const icon = { paths: ["M4 4 20 20"] };
const addon: AddonContribution = {
  addonId: "connectors", enabled: true, settings: [], values: {}, commands: [],
  tools: [{ id: "connect", label: "connect", kind: "connect", snap: "ports", settings: [], icon, stroke: true }],
  strings: {
    en: { name: "Connectors", connect: "Connect", a: "Short", b: "Second" },
    de: { name: "Verbinder", connect: "Verbinden", a: "Kurz", b: "Sehr lange Einstellung" },
  },
};

describe("addon tool settings", () => {
  it("seeds an independent slot from the pen held on first use", () => {
    const pen = DEFAULT_SETTINGS.penPresets[1];
    const slot = defaultAddonStrokeSlot(DEFAULT_SETTINGS, 1);
    expect(slot.style).toMatchObject({ strokeColor: pen.color, strokeWidthPt: pen.widthPt, pattern: pen.pattern, opacity: 1 });
    expect(slot.widths).toEqual(DEFAULT_SETTINGS.penWidths);
    expect(slot.widths).not.toBe(DEFAULT_SETTINGS.penWidths);
    expect(slot.colors).not.toBe(DEFAULT_SETTINGS.penSwatches);
  });

  it("chooses a single presentation from the available icons and translated lengths", () => {
    const field: AddonField = { id: "mode", label: "connect", type: "choice", default: "a",
      choices: [{ value: "a", label: "a", icon }, { value: "b", label: "b", icon }] };
    expect(toolFieldPresentation(addon, field, "de")).toBe("icons");
    const mixed = { ...field, choices: [field.choices[0], { value: "b", label: "b" }] };
    expect(toolFieldPresentation(addon, mixed, "en")).toBe("text-segments");
    expect(toolFieldPresentation(addon, mixed, "de")).toBe("list");
    const six = { ...field, choices: Array.from({ length: 6 }, (_, i) => ({ value: String(i), label: "a", icon })) };
    expect(toolFieldPresentation(addon, six, "en")).toBe("list");
  });

  it("uses the card on a side dock, in a narrow window, or when segments cannot fit", () => {
    expect(fitsAddonSegments("bottom", 1280, 626, 205)).toBe(true);
    expect(fitsAddonSegments("bottom", 900, 626, 205)).toBe(false);
    expect(fitsAddonSegments("left", 1280, 626, 205)).toBe(false);
    expect(fitsAddonSegments("top", 1280, 1100, 205)).toBe(false);
  });

  it("writes enabled addon visibility with the shared palette preference", () => {
    const groups = settingGroups(undefined, [], false, [], [addon, { ...addon, addonId: "disabled", enabled: false }], "de");
    const palette = groups.find(group => group.id === "palette")!;
    const items = palette.items.filter(item => item.section);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ label: "Verbinden", hint: "Verbinder" });
    const control = items[0].control;
    if (control.kind !== "toggle") throw new Error("Expected a visibility toggle");
    const hidden = control.write(DEFAULT_SETTINGS, false);
    expect(hidden.hiddenTools).toContain("addon:connectors/connect");
    expect(control.read(hidden)).toBe(false);
    expect(control.write(hidden, true).hiddenTools).toEqual([]);
  });
});
