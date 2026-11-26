<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { invoke } from "@tauri-apps/api/core";
  import { formatNumber, t } from "../i18n";
  let { beta, restart, autoCheck = false }: { beta: boolean; autoCheck?: boolean; restart?: () => Promise<void> } = $props();
  let status = $state<"idle" | "checking" | "current" | "local" | "failed" | "staging" | "ready" | "stageFailed">("idle");
  let offer = $state<{ version: string; size: number; beta: boolean } | null>(null);
  let installed = $state("");
  onMount(() => { if ("__TAURI_INTERNALS__" in window) void getVersion().then(v => { installed = v; }).catch(() => {}); });
  onMount(() => { if (autoCheck) void check(); });
  async function check() {
    status = "checking";
    offer = null;
    try {
      offer = await invoke("check_app_release", { beta });
      status = offer ? "idle" : "current";
    } catch (error) { status = error === "local" ? "local" : "failed"; }
  }
  async function download() {
    if (!offer) return;
    status = "staging";
    try {
      await invoke("stage_app_release", { version: offer.version, beta });
      status = "ready";
    } catch { status = "stageFailed"; }
  }
</script>
{#if installed}<p>{$t("appUpdates.installed", { version: installed })}</p>{/if}
<button class="run" type="button" onclick={check} disabled={status === "checking" || status === "staging"}>{$t("appUpdates.check")}</button>
<p role="status">
  {#if status === "staging" || status === "stageFailed"}{$t(`appUpdates.${status}`)}
  {:else if offer}
    {$t("appUpdates.available", { version: offer.version, channel: $t(offer.beta ? "appUpdates.betaChannel" : "appUpdates.stableChannel"), size: formatNumber(Math.ceil(offer.size / 1048576)) })}
  {:else if status !== "idle" && status !== "ready"}{$t(`appUpdates.${status}`)}{/if}
</p>
{#if offer && status === "ready"}
  <button class="run" type="button" onclick={restart}>{$t("appUpdates.restart")}</button>
{:else if offer && status !== "staging"}
  <button class="run" type="button" onclick={download}>{$t("appUpdates.download")}</button>
{/if}
