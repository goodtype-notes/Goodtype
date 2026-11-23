import { invoke } from "@tauri-apps/api/core";
import type { PageTemplate, ShapeGeometry, ShapeStyle } from "../../model";
import type { HistoryResult } from "../../ipc/types";

export type PackShape = {
  id: string;
  label: string;
  search: string;
  widthPt: number;
  heightPt: number;
  objects: (
    | { type: "shape"; x: number; y: number; geometry: ShapeGeometry; style: Partial<ShapeStyle> }
    | { type: "typst"; x: number; y: number; source: string; layoutWidthPt: number; scale: number }
  )[];
};

export type ContentPack = {
  id: string;
  strings: { en: Record<string, string>; de: Record<string, string> };
  shapes: { id: string; label: string; items: PackShape[] }[];
  templates: { label: string; template: PageTemplate }[];
  palettes: { id: string; label: string; colors: string[] }[];
  themes: { id: string; label: string; tokens: Record<string, string> }[];
  typstPackages: string[];
};

export const contentPacks = (id?: string, includeDisabled = false) => invoke<ContentPack[]>("addon_content", { id, includeDisabled });

export type ShapeChoice = { addonId: string; libraryId: string; itemId: string };
export type ShapePlacement = ShapeChoice & {
  scale: number;
  point?: { x: number; y: number };
  penColor: string;
  penWidthPt: number;
};

export const insertContentShape = (root: string, pageId: string, request: ShapePlacement & { revision: number }) =>
  invoke<HistoryResult & { selectedIds: string[] }>("insert_content_shape", { root, pageId, request });

export function packText(pack: ContentPack, key: string, language: string): string {
  return (language === "de" ? pack.strings.de[key] : undefined) ?? pack.strings.en[key] ?? key;
}

export function packKind(pack: ContentPack | undefined) {
  if (!pack) return "content";
  const kinds = (["shapes", "templates", "palettes", "theme"] as const).filter(kind =>
    (kind === "theme" ? pack.themes : pack[kind]).length > 0,
  );
  return kinds.length === 1 && !pack.typstPackages.length ? kinds[0] : "content";
}

export function shapeLibraries(packs: ContentPack[], language: string) {
  return packs.flatMap(pack => pack.shapes.map(library => ({
    id: [pack.id, library.id].join("."),
    label: packText(pack, library.label, language),
    items: library.items.map(item => ({
      choice: { addonId: pack.id, libraryId: library.id, itemId: item.id },
      item,
      label: packText(pack, item.label, language),
      search: packText(pack, item.search, language),
    })),
  })));
}

export type ShapeLibraryModel = ReturnType<typeof shapeLibraries>;
export function searchShapes(libraries: ShapeLibraryModel, libraryId: string, query: string, language: string) {
  const words = query.toLocaleLowerCase(language).trim().split(/\s+/);
  return libraries.filter(library => !libraryId || library.id === libraryId).flatMap(library =>
    library.items.filter(shape => {
      const text = [shape.label, shape.search, library.label].join(" ").toLocaleLowerCase(language);
      return words.every(word => text.includes(word));
    }),
  );
}

export function installedThemes(packs: ContentPack[], language: string) {
  return packs.flatMap(pack => pack.themes.map(theme => ({
    value: [pack.id, theme.id].join("."),
    label: packText(pack, theme.label, language),
    tokens: theme.tokens,
  })));
}

export function applyTheme(style: Pick<CSSStyleDeclaration, "setProperty" | "removeProperty">, tokens: Record<string, string>) {
  for (const [key, value] of Object.entries(tokens)) style.setProperty(key, value);
  return () => {
    for (const key of Object.keys(tokens)) style.removeProperty(key);
  };
}
