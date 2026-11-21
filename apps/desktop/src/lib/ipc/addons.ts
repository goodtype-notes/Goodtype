import { invoke } from "@tauri-apps/api/core";
import type { en } from "../i18n/en";

type AddonMessages = typeof en.addons;
type PrefixWith<Key extends string> = {
  [Id in keyof AddonMessages]: Key extends keyof AddonMessages[Id] ? `addons.${Id}` : never;
}[keyof AddonMessages];

export type ImportAddon = {
  id: string;
  kind: "import";
  i18nPrefix: PrefixWith<"fileHelp">;
  extensions: string[];
  artwork: boolean;
};

export type RecognitionAddon = {
  id: string;
  kind: "recognize";
  recognizes: string;
  i18nPrefix: PrefixWith<"label">;
};

type Addon = ImportAddon | RecognitionAddon;

let catalog: Promise<Addon[]> | undefined;

export function addonCatalog(): Promise<Addon[]> {
  return catalog ??= invoke<Addon[]>("addon_catalog").catch((error) => {
    catalog = undefined;
    throw error;
  });
}

export async function importAddons(): Promise<ImportAddon[]> {
  return (await addonCatalog()).filter((addon) => addon.kind === "import");
}

export async function recognitionAddon(): Promise<RecognitionAddon> {
  const addon = (await addonCatalog()).find(
    (addon): addon is RecognitionAddon => addon.kind === "recognize" && addon.recognizes === "math",
  );
  if (!addon) throw new Error("Recognition addon is missing from the catalog");
  return addon;
}
