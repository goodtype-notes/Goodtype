import { addonText, type ManagedAddon } from "../../ipc/addons";

export function matchesAddon(addon: Pick<ManagedAddon, "name">, query: string, language: string) {
  return addonText(addon.name, language).toLocaleLowerCase(language).includes(query.trim().toLocaleLowerCase(language));
}

export function addonAction(addon: ManagedAddon) {
  if (addon.activity === "downloading" || addon.activity === "installing") return "cancel";
  if (addon.approval || ["checking", "awaitingApproval", "importing", "publishing"].includes(addon.activity)) return null;
  if (addon.update) return "update";
  if (addon.release && addon.status === "needs-reinstall") return "reinstall";
  if (addon.release && addon.status === "available") return "install";
  return null;
}

export function downloadPercent(progress: ManagedAddon["progress"]) {
  return progress && progress[1] > 0 ? Math.min(100, Math.round(progress[0] / progress[1] * 100)) : null;
}
