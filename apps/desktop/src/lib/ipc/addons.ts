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

export async function mathRecognitionAddon(): Promise<RecognitionAddon | undefined> {
  return (await addonCatalog()).find(
    (addon): addon is RecognitionAddon => addon.kind === "recognize" && addon.recognizes === "math",
  );
}

export type AddonPermission =
  | "picked-import-file" | "derived-import-artwork" | "publish-import-batch"
  | "selected-ink" | "add-page-objects";

type Release = { version: string; sizeBytes: number };

export type ManagedAddon = {
  id: string;
  name: { en: string; de: string };
  installed: string | null;
  status: "unsupported-os" | "unpublished" | "available" | "enabled" | "disabled" | "incompatible-api" | "verification-failed";
  expectedApi: string;
  foundApi: string | null;
  permissions: AddonPermission[];
  catalogPermissions: AddonPermission[];
  formats: string[];
  memoryBytes: number;
  timeoutSeconds: number;
  outputBytes: number;
  release: Release | null;
  update: Release | null;
  updateChecked: boolean;
  updateFailed: boolean;
  canCheckUpdate: boolean;
  directApply: boolean;
  activity: "idle" | "checking" | "downloading" | "installing" | "installFailed" | "updateIncompatible" | "importing" | "publishing" | "finished" | "importFailed";
  progress: [number, number] | null;
};

export type AddonManager = { allowDirectApply: boolean; addons: ManagedAddon[] };

export const listAddons = () => invoke<AddonManager>("addon_list");
export const setAddonEnabled = (id: string, enabled: boolean) => invoke<void>("addon_set_enabled", { id, enabled });
export const removeAddon = (id: string) => invoke<void>("addon_remove", { id });
export const setAddonDirectApply = (id: string | null, allowed: boolean) =>
  invoke<void>("addon_set_direct_apply", { id, allowed });
