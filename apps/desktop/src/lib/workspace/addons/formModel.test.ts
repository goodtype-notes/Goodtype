import { afterEach, expect, it } from "vitest";
import { render } from "svelte/server";
import { setLanguage } from "../../i18n";
import type { AddonContribution, AddonField } from "../../ipc/addons";
import AddonFieldControl from "./AddonField.svelte";
import AddonSettings from "./AddonSettings.svelte";
import { accepts, formDefaults, matchesField } from "./formModel";

const fields: AddonField[] = [
  { type: "text", id: "title", label: "title", maxLength: 4, default: "ab" },
  { type: "number", id: "count", label: "count", min: 1, max: 9, step: 2, default: 3 },
  { type: "choice", id: "color", label: "color", default: "blue", choices: [{ value: "blue", label: "blue" }, { value: "green", label: "green" }] },
  { type: "toggle", id: "outline", label: "outline", default: false, search: "contour" },
];

const addon: AddonContribution = {
  addonId: "fixture", enabled: true, settings: fields.slice(3), commands: [], values: { outline: true },
  strings: {
    en: { name: "Diagram tools", title: "Title", count: "Count", color: "Color", blue: "Blue", green: "Green", outline: "Show outlines", contour: "Contours" },
    de: { name: "Diagrammwerkzeuge", title: "Titel", count: "Anzahl", color: "Farbe", blue: "Blau", green: "Gruen", outline: "Konturen anzeigen", contour: "Umrisse" },
  },
};

afterEach(() => setLanguage("en"));

it("matches Rust defaults, Unicode lengths, number steps and declared values", () => {
  expect(formDefaults(fields)).toEqual({ title: "ab", count: 3, color: "blue", outline: false });
  expect(accepts(fields[0], "abcd")).toBe(true);
  expect(accepts(fields[0], "abcde")).toBe(false);
  expect(accepts(fields[0], "a\nb")).toBe(false);
  expect(accepts(fields[1], 9)).toBe(true);
  expect(accepts(fields[1], 4)).toBe(false);
  expect(accepts(fields[1], 11)).toBe(false);
  expect(accepts(fields[2], "missing")).toBe(false);
  expect(accepts(fields[3], "true")).toBe(false);
});

it("renders native keyboard controls and localized searchable settings", () => {
  const choice = render(AddonFieldControl, { props: { addon, field: fields[2], value: "blue", onChange: () => {} } }).body;
  expect(choice).toContain('type="radio"');
  expect(choice).toContain("Blue");
  expect(matchesField(addon, fields[3], "contours", "en")).toBe(true);
  expect(matchesField(addon, fields[3], "umrisse", "en")).toBe(false);
  setLanguage("de");
  const settings = render(AddonSettings, { props: { addon, query: "umrisse" } }).body;
  expect(settings).toContain("Konturen anzeigen");
  expect(settings).toContain('role="switch"');
  expect(settings).toContain('aria-checked="true"');
});
