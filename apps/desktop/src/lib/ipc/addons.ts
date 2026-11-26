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

type Addon = ImportAddon | RecognitionAddon | { id: string; kind: "content" | "objects"; i18nPrefix: PrefixWith<"label"> };

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
  | "selected-ink" | "add-page-objects" | "read-pages" | "own-objects" | "react-to-changes";

type Release = { version: string; sizeBytes: number };

export type AddonApproval = { token: string; version: string; permissions: AddonPermission[] };

export type ManagedAddon = {
  id: string;
  name: { en: string; de: string };
  summary: AddonName | null;
  repository: string;
  /** Older catalogs supply kind through addon_catalog. */
  kind?: string;
  needsNewerApp?: boolean;
  minGoodtype?: string | null;
  missingFeatures?: string | null;
  problem?: "missingFeatures" | "download" | "verify" | "probe" | "policy" | "unavailable" | null;
  installed: string | null;
  previous: string | null;
  status: "missing-features" | "unsupported-os" | "unpublished" | "available" | "enabled" | "disabled" | "incompatible-api" | "verification-failed" | "needs-reinstall";
  expectedApi: string;
  foundApi: string | null;
  permissions: AddonPermission[];
  catalogPermissions: AddonPermission[];
  formats: string[];
  memoryBytes: number;
  timeoutSeconds: number;
  outputBytes: number;
  /** "sandbox": AppContainer, no network, own files only. "job": user access, memory/time limits. */
  isolation: "sandbox" | "job" | "content";
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

export type RegistryProblem = "notFound" | "offline" | "status" | "signature" | "expired" | "rollback" | "invalid";
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
export async function setAddonDirectApply(id: string | null, allowed: boolean) {
  await invoke<void>("addon_set_direct_apply", { id, allowed });
  window.dispatchEvent(new Event(ADDON_AVAILABILITY_CHANGED));
}

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
  anchor: { x: number; y: number; width: number; height: number } | null;
  /** Unsupported math: the recognized LaTeX, shown for correction. */
  latex: string | null;
  kind: "blocks" | "pathChoice" | "pathStart" | "pathEnd";
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

export type AddonValue = string | number | boolean;
export type AddonValues = Record<string, AddonValue>;
type AddonIcon = { paths: string[] };
type FieldLabel = { id: string; label: string; hint?: string | null; search?: string | null };
export type AddonField = FieldLabel & (
  | { type: "toggle"; default: boolean }
  | { type: "choice"; default: string; choices: { value: string; label: string; icon?: AddonIcon }[] }
  | { type: "text"; default: string; maxLength: number; pattern?: "any" | "number" | "identifier" | null }
  | { type: "number"; default: number; min: number; max: number; step: number }
);
export type AddonTool = {
  id: string; label: string; icon: AddonIcon; stroke?: boolean;
  kind: "place" | "connect" | "frame"; snap: "ports" | "none"; settings: string[];
};
export type AddonPoint = { x: number; y: number; attach: { objectId: string; port: number } | null };
export type AddonObjectAction =
  | { kind: "tool"; id: string; points: AddonPoint[]; style?: import("../model").ShapeStyle; pen?: { strokeColor: string; strokeWidthPt: number } }
  | { kind: "command"; id: string; selected: string[] }
  | { kind: "pathEdit"; id: string; objectId: string; edit: PathEdit };
export type PathEdit =
  | { kind: "endpoint"; end: "start" | "end"; point: { x: number; y: number }; attach: AddonPoint["attach"] }
  | { kind: "choice"; value: string };
export const addonObjectAction = (root: string, pageId: string, revision: number, addonId: string, action: AddonObjectAction, requestId?: string) =>
  invoke<import("./types").HistoryResult | { review: AddonReview; direct: boolean } | null>(
    "addon_object_action", { request: { root, pageId, revision, addonId, action, requestId } });

export const prepareAddonReactions = (root: string, pageId: string, revision: number, baseRevision: number, retry: boolean) =>
  invoke<{ token: string | null; retry: boolean; failed: AddonName | null }>("addon_react", { root, pageId, revision, baseRevision, retry });
export const applyAddonReaction = (token: string) =>
  invoke<[import("./types").HistoryResult | null, boolean]>("addon_apply_reaction", { token });

export type AddonContribution = {
  addonId: string;
  enabled: boolean;
  directApply?: boolean;
  commands: ContributionCommand[];
  tools?: AddonTool[];
  strings: Record<"en" | "de", Record<string, string>>;
  settings: AddonField[];
  values: AddonValues;
};
export type ContributionCommand = { id: string; label: string } & (
  | { surface: "insert-form" | "page-generator"; form: AddonField[] }
  | { surface: "own-object"; pathEdit?: { choice: string; snap: "ports" } }
  | { surface: "export"; format: { extension: string; label: string; assets: boolean } }
);
export type ContributionEntry = { addon: AddonContribution; command: ContributionCommand };
export const contributionText = (addon: AddonContribution, key: string, language: string) =>
  addon.strings[language === "de" ? "de" : "en"][key] ?? addon.strings.en[key] ?? "";
export const addonContributions = () => invoke<AddonContribution[]>("addon_contributions");
export const setAddonField = (id: string, field: string, value: AddonValue) =>
  invoke<void>("addon_set_field", { id, field, value });

type FormRequest = {
  root: string; pageId: string; addonId: string; command: string; requestId: string;
  form: AddonValues; reviewFirst: boolean; geometry: import("../model").PageGeometry | null; anchor: [number, number];
};
export type FormPreview = {
  id: string; direct: boolean; geometry: import("../model").PageGeometry;
  pages: { background: import("../model").PageBackground; items: import("./helper").ProposalItem[] }[];
};
export const previewAddonForm = (request: FormRequest) => invoke<FormPreview>("addon_form_preview", { request });
export const dismissAddonForm = (id: string) => invoke<void>("addon_form_dismiss", { id });
export const insertAddonForm = (root: string, pageId: string, id: string, direct: boolean) =>
  invoke<import("./types").HistoryResult | null>("addon_form_insert", { root, pageId, id, direct });
export const addGeneratedPages = (root: string, id: string, position: import("../model").PagePosition) =>
  invoke<import("./types").StructureHistoryResult>("addon_form_add_pages", { root, id, position });
export const exportWithAddon = (root: string, pageId: string | null, entry: ContributionEntry, language: string) =>
  invoke<boolean>("addon_export", { request: { root, pageId, addonId: entry.addon.addonId, command: entry.command.id, language } });
