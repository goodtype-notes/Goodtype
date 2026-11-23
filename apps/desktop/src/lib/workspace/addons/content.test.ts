import { describe, expect, it } from "vitest";
import { applyTheme, installedThemes, packKind, searchShapes, shapeLibraries, type ContentPack } from "./content";
import { settingGroups } from "../settingsSchema";
import { DEFAULT_SETTINGS } from "../../settings";

const pack: ContentPack = {
  id: "engineering-shapes",
  strings: {
    en: { lib: "Circuits", resistor: "Resistor", keywords: "electric circuit", desk: "Midnight" },
    de: { lib: "Schaltungen", resistor: "Widerstand", keywords: "elektrisch Schaltkreis", desk: "Mitternacht" },
  },
  shapes: [{ id: "circuits", label: "lib", items: [
    { id: "r", label: "resistor", search: "keywords", widthPt: 48, heightPt: 16, objects: [] },
  ] }],
  templates: [],
  palettes: [],
  themes: [{ id: "desk", label: "desk", tokens: { "--panel": "#252735" } }],
  typstPackages: [],
};

describe("content packs", () => {
  it("searches the active language, library, item name and keywords", () => {
    const libraries = shapeLibraries([pack], "de");
    expect(libraries[0]?.label).toBe("Schaltungen");
    expect(searchShapes(libraries, "", "  WIDER elektrisch ", "de").map(row => row.choice)).toEqual([
      { addonId: "engineering-shapes", libraryId: "circuits", itemId: "r" },
    ]);
    expect(searchShapes(libraries, "", "Resistor", "de")).toEqual([]);
    expect(searchShapes(libraries, "other", "Widerstand", "de")).toEqual([]);
    expect(packKind({ ...pack, themes: [] })).toBe("shapes");
    expect(packKind(pack)).toBe("content");
  });

  it("applies a selected theme and restores CSS defaults when removed", () => {
    const [theme] = installedThemes([pack], "de");
    expect(theme.label).toBe("Mitternacht");
    const group = settingGroups(undefined, [], false, [theme]).find(group => group.id === "appearance");
    const control = group?.items.find(item => item.id === "theme")?.control;
    if (control?.kind !== "select") throw new Error("Theme selector missing");
    const settings = control.write(DEFAULT_SETTINGS, theme.value);
    expect(control.read(settings)).toBe(theme.value);

    const properties = new Map<string, string>();
    const clear = applyTheme({
      setProperty: (key, value) => void properties.set(key, value ?? ""),
      removeProperty: key => {
        properties.delete(key);
        return "";
      },
    }, theme.tokens);
    expect(properties.get("--panel")).toBe("#252735");
    clear();
    expect(properties.size).toBe(0);
    const removedControl = settingGroups()[0].items.find(item => item.id === "theme")?.control;
    if (removedControl?.kind !== "select") throw new Error("Theme selector missing");
    expect(removedControl.read(settings)).toBe("");
  });
});
