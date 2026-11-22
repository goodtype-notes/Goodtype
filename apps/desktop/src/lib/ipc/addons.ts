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

export type AddonApproval = { token: string; version: string; permissions: AddonPermission[] };

export type ManagedAddon = {
  id: string;
  name: { en: string; de: string };
  summary: AddonName | null;
  repository: string;
  /** Older catalogs supply kind through addon_catalog. */
  kind?: string;
  problem?: "download" | "verify" | "probe" | "policy" | "unavailable" | null;
  installed: string | null;
  previous: string | null;
  status: "unsupported-os" | "unpublished" | "available" | "enabled" | "disabled" | "incompatible-api" | "verification-failed" | "needs-reinstall";
  expectedApi: string;
  foundApi: string | null;
  permissions: AddonPermission[];
  catalogPermissions: AddonPermission[];
  formats: string[];
  memoryBytes: number;
  timeoutSeconds: number;
  outputBytes: number;
  /** "sandbox": AppContainer, no network, own files only. "job": user access, memory/time limits. */
  isolation: "sandbox" | "job";
  release: Release | null;
  update: Release | null;
  directApply: boolean;
  activity: "awaitingApproval" | "idle" | "checking" | "downloading" | "installing" | "installFailed" | "updateIncompatible" | "importing" | "publishing" | "finished" | "importFailed";
  approval: AddonApproval | null;
  progress: [number, number] | null;
};

export type Span = { text: string; strong: boolean; em: boolean; code: boolean; href: string | null };
export type Block =
  | { kind: "heading"; level: 2 | 3; spans: Span[] }
  | { kind: "paragraph"; spans: Span[] }
  | { kind: "list"; ordered: boolean; start: number | null; items: Block[][] }
  | { kind: "code"; text: string }
  | { kind: "quote"; blocks: Block[] }
  | { kind: "rule" }
  | { kind: "table"; head: Span[][]; rows: Span[][][] };

export type AddonDetails = {
  readme: Block[] | null;
  preview: string | null;
  origin: "cache" | "network" | null;
  reference: string | null;
  verified: boolean;
  problem: "offline" | "invalid" | "unavailable" | null;
};

export const addonDetails = (id: string) => invoke<AddonDetails>("addon_details", { id });
export const installAddon = (id: string, approved: AddonPermission[]) =>
  invoke<void>("addon_install", { id, approved });
export const updateAddon = (id: string, approved: AddonPermission[]) =>
  invoke<void>("addon_update", { id, approved });

export type RegistryProblem = "offline" | "status" | "signature" | "expired" | "rollback" | "invalid";
export type RegistryCheck = { lastCheck: number | null; checking: boolean; problem: RegistryProblem | null; expired: boolean };
export type AddonManager = {
  allowDirectApply: boolean;
  recovery: boolean;
  autoCheck: boolean;
  registry: RegistryCheck;
  addons: ManagedAddon[];
};

export const checkAddonRegistry = (mode: "startup" | "manager" | "manual") =>
  invoke<void>("addon_check_registry", { mode });
export const setAddonAutoCheck = (enabled: boolean) =>
  invoke<void>("addon_set_auto_check", { enabled });

export const listAddons = () => invoke<AddonManager>("addon_list");
export const ADDON_AVAILABILITY_CHANGED = "goodtype:addon-availability-changed";
export async function setAddonEnabled(id: string, enabled: boolean) {
  await invoke<void>("addon_set_enabled", { id, enabled });
  window.dispatchEvent(new Event(ADDON_AVAILABILITY_CHANGED));
}

export async function removeAddon(id: string) {
  await invoke<void>("addon_remove", { id });
  window.dispatchEvent(new Event(ADDON_AVAILABILITY_CHANGED));
}
export async function rollbackAddon(id: string) {
  await invoke<void>("addon_rollback", { id });
  window.dispatchEvent(new Event(ADDON_AVAILABILITY_CHANGED));
}
export const setAddonDirectApply = (id: string | null, allowed: boolean) =>
  invoke<void>("addon_set_direct_apply", { id, allowed });

export const resetAddonPolicy = () => invoke<void>("addon_reset_policy");
export const approveAddon = (id: string, token: string) => invoke<void>("addon_approve", { id, token });

/** Manifest strings: plain text, English for any language without its own table. */
export type AddonName = { en: string; de: string };
export const addonText = (name: AddonName, language: string) => (language === "de" ? name.de : name.en) || name.en;

export type SelectionCommand = { addonId: string; command: string; label: AddonName };
export const addonSelectionCommands = () => invoke<SelectionCommand[]>("addon_selection_commands");

/** An addon-owned proposal. Identity, version and status belong to Rust (ADR 0045). */
export type AddonReview = {
  id: string;
  version: number;
  root: string;
  pageId: string;
  status: "pending";
  addon: AddonName;
  items: import("./helper").ProposalItem[];
  anchor: { x: number; y: number; width: number; height: number };
  /** Unsupported math: the recognized LaTeX, shown for correction. */
  latex: string | null;
  kind: "blocks";
};

export type AddonSelection = {
  root: string;
  pageId: string;
  strokeIds: string[];
  addonId: string;
  command: string;
  requestId: string;
  /** False only for a command the writer started; Rust still checks both direct-apply toggles. */
  reviewFirst: boolean;
};
export const invokeAddonSelection = (selection: AddonSelection) =>
  invoke<{ review: AddonReview; direct: boolean }>("invoke_addon_selection", { selection });
export const insertAddonProposal = (root: string, pageId: string, proposalId: string, proposalVersion: number, direct: boolean) =>
  invoke<import("./types").HistoryResult | null>("insert_addon_proposal", { root, pageId, proposal: { proposalId, proposalVersion }, direct });
export const editAddonProposal = (id: string, version: number, sources: string[]) =>
  invoke<AddonReview>("addon_edit", { id, version, sources });
export const dismissAddonProposal = (id: string) => invoke<void>("addon_dismiss", { id });
export const cancelAddonCommand = (requestId: string) => invoke<void>("addon_cancel", { requestId });
