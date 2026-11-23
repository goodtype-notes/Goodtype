<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import PackShapePreview from "./PackShapePreview.svelte";
  import { contentPacks, packKind, packText, shapeLibraries, type ContentPack } from "./content";
  import { displayLocale, formatNumber, locale, t, type MessageKey } from "../../i18n";
  import {
    ADDON_AVAILABILITY_CHANGED, addonCatalog, addonContributions, addonDetails, addonText, checkAddonRegistry,
    installAddon, listAddons, removeAddon, rollbackAddon, setAddonDirectApply, setAddonEnabled,
    updateAddon, type AddonContribution, type AddonDetails, type AddonManager, type ManagedAddon,
  } from "../../ipc/addons";
  import { cancelImporter, megabytes } from "../../ipc/importer";
  import AddonApproval from "../AddonApproval.svelte";
  import AddonPermissions from "../AddonPermissions.svelte";
  import ConfirmDialog from "../ConfirmDialog.svelte";
  import AddonSettings from "./AddonSettings.svelte";
  import AddonPreview from "./AddonPreview.svelte";
  import ReadmeView from "./ReadmeView.svelte";
  import { addonAction, downloadPercent, matchesAddon } from "./storeModel";

  let { onClose }: { onClose: () => void } = $props();
  let contributions = $state.raw<AddonContribution[]>([]);
  let manager = $state.raw<AddonManager>();
  let packs = $state.raw<ContentPack[]>([]);
  let sideloadFailed = $state(false);
  let sideloadPending = $state(false);
  const dev = import.meta.env.DEV;

  async function refreshPacks() {
    try { packs = await contentPacks(undefined, true); }
    catch { /* Keep the last successful result. */ }
  }
  async function loadLocalPack() {
    sideloadPending = true;
    sideloadFailed = false;
    try {
      if (await invoke<boolean>("addon_sideload_content", { uiLanguage: $locale })) {
        await refresh();
        await refreshPacks();
        window.dispatchEvent(new Event(ADDON_AVAILABILITY_CHANGED));
      }
    } catch {
      sideloadFailed = true;
    } finally {
      sideloadPending = false;
    }
  }
  let kinds = $state<Record<string, string>>({});
  let query = $state("");
  let selected = $state<string>();
  let details = $state<Record<string, AddonDetails>>({});
  let loading = $state<Record<string, boolean>>({});
  let errors = $state<Record<string, { key: MessageKey; retry: () => void } | undefined>>({});
  let pending = $state<Record<string, boolean>>({});
  let cancelling = $state<Record<string, boolean>>({});
  let listFailed = $state(false);
  let checkFailed = $state(false);
  let checking = $state(false);
  let confirming = $state(false);
  let initialFocus: HTMLButtonElement;
  let detailHeading = $state<HTMLHeadingElement>();
  let scroller: HTMLElement;
  let gridScroll = 0;
  let tileButtons: Record<string, HTMLButtonElement> = {};
  let prompt = $state<{ addon: ManagedAddon; update: boolean; trigger: HTMLButtonElement }>();
  let permissionPopup = $state<HTMLDivElement>();
  let confirmInstall = $state<HTMLButtonElement>();
  let request = 0;

  const registry = $derived(manager?.registry);
  const registryText = $derived(checking || registry?.checking ? $t("addons.manager.activity.checking")
    : registry?.problem ? $t(`addons.manager.registryProblem.${registry.problem}`)
    : registry?.expired ? $t("addons.manager.outdated")
    : checkFailed ? $t("addons.manager.checkFailed")
    : registry?.lastCheck != null ? $t("addons.store.checked", {
      date: new Intl.DateTimeFormat($displayLocale, { dateStyle: "medium", timeStyle: "short" }).format(registry.lastCheck * 1000),
    }) : "");
  const visible = $derived(manager?.addons.filter(addon => matchesAddon(addon, query, $locale)) ?? []);
  const active = $derived(manager?.addons.find(addon => addon.id === selected));
  const activePack = $derived(packs.find(pack => pack.id === selected));
  const activeShapes = $derived(activePack ? shapeLibraries([activePack], $locale).flatMap(library => library.items) : []);
  const activeDetails = $derived(selected ? details[selected] : undefined);
  const newPermissions = $derived(prompt?.addon.catalogPermissions.filter(permission =>
    !prompt?.update || !prompt.addon.permissions.includes(permission)) ?? []);
  const summaryKeys = {
    goodnotes: "addons.goodnotes.description", onenote: "addons.onenote.description", math: "addons.math.description",
    connectors: "addons.connectors.description",
    "engineering-shapes": "addons.engineeringShapes.description", "study-templates": "addons.studyTemplates.description",
  } as const;

  const icons = {
    back: "m10 5-7 7 7 7M3 12h17",
    update: "M20 4v5h-5M4 20v-5h5M5 9a8 8 0 0 1 13-3l2 3m-1 6a8 8 0 0 1-13 3l-2-3",
    install: "M12 3v12m-5-5 5 5 5-5M4 15v5h16v-5",
    cancel: "m6 6 12 12M6 18 18 6",
    check: "m5 12 4 4L19 6",
  };

  async function refresh() {
    const started = ++request;
    try {
      const next = await listAddons();
      if (started !== request) return;
      const changed = manager && next.addons.some(addon => {
        const previous = manager?.addons.find(item => item.id === addon.id);
        return previous && (previous.installed !== addon.installed || previous.status !== addon.status);
      });
      manager = next;
      if (sideloadPending) {
        const approval = next.addons.find((addon) => addon.approval);
        if (approval && selected !== approval.id) void openDetail(approval);
      }
      if (changed) {
        void refreshPacks();
        window.dispatchEvent(new Event(ADDON_AVAILABILITY_CHANGED));
        void loadFields();
      }
      listFailed = false;
    } catch {
      if (started === request) listFailed = true;
    }
  }

  async function check(mode: "manual" | "manager") {
    checking = true;
    checkFailed = false;
    try { await checkAddonRegistry(mode); }
    catch { checkFailed = true; }
    finally { await refresh(); checking = false; }
  }

  onMount(() => {
    initialFocus.focus();
    void refresh();
    void refreshPacks();
    void check("manager");
    void addonCatalog().then(catalog => {
      kinds = Object.fromEntries(catalog.map(addon => [addon.id, addon.kind]));
    }).catch(() => {});
    // Rust owns the operation state, including downloads started before this surface opened.
    const timer = setInterval(() => void refresh(), 650);
    return () => clearInterval(timer);
  });

  function summary(addon: ManagedAddon, language: string) {
    const key = summaryKeys[addon.id as keyof typeof summaryKeys];
    return (addon.summary && addonText(addon.summary, language)) || (key ? $t(key) : "");
  }

  async function loadFields() {
    try { contributions = await addonContributions(); }
    catch { contributions = []; }
  }

  async function openDetail(addon: ManagedAddon) {
    gridScroll = scroller.scrollTop;
    selected = addon.id;
    void loadDetails(addon.id);
    void loadFields();
    await tick();
    scroller.scrollTop = 0;
    detailHeading?.focus();
  }

  async function loadDetails(id: string) {
    if (loading[id] || (details[id] && !details[id].problem)) return;
    loading[id] = true;
    try { details[id] = await addonDetails(id); }
    catch { details[id] = { readme: null, preview: null, origin: null, reference: null, verified: false, problem: "unavailable" }; }
    finally { loading[id] = false; }
  }

  async function back() {
    if (prompt) { closePrompt(); return; }
    if (!selected) { onClose(); return; }
    const id = selected;
    selected = undefined;
    await tick();
    scroller.scrollTop = gridScroll;
    (tileButtons[id] ?? initialFocus).focus({ preventScroll: true });
  }

  function escape(event: KeyboardEvent) {
    if (event.key !== "Escape" || event.defaultPrevented || confirming) return;
    event.preventDefault();
    void back();
  }

  async function run(addon: ManagedAddon, action: () => Promise<void>, key: MessageKey = "addons.manager.actionFailed", retry: () => void = () => void run(addon, action, key)) {
    if (pending[addon.id]) return;
    pending[addon.id] = true;
    errors[addon.id] = undefined;
    try { await action(); }
    catch { errors[addon.id] = { key, retry }; }
    finally { await refresh(); pending[addon.id] = false; }
  }

  function start(addon: ManagedAddon, update: boolean) {
    const command = update ? updateAddon : installAddon;
    void run(addon, () => command(addon.id, addon.catalogPermissions), "addons.manager.actionFailed", () => retryInstall(addon));
  }

  async function cancel(addon: ManagedAddon) {
    if (cancelling[addon.id]) return;
    cancelling[addon.id] = true;
    errors[addon.id] = undefined;
    try { await cancelImporter(addon.id); }
    catch { errors[addon.id] = { key: "addons.manager.actionFailed", retry: () => void cancel(addon) }; }
    finally { await refresh(); cancelling[addon.id] = false; }
  }

  async function choose(event: MouseEvent & { currentTarget: HTMLButtonElement }, addon: ManagedAddon) {
    event.stopPropagation();
    const trigger = event.currentTarget;
    const action = addonAction(addon);
    if (action === "cancel") { void cancel(addon); return; }
    if (!action || pending[addon.id]) return;
    if (action === "update" && addon.catalogPermissions.every(permission => addon.permissions.includes(permission))) {
      start(addon, true);
      return;
    }
    prompt = { addon, update: action === "update", trigger };
    await tick();
    if (!prompt || !permissionPopup) return;
    permissionPopup.showPopover();
    placePrompt();
    confirmInstall?.focus({ preventScroll: true });
  }

  function placePrompt() {
    if (!prompt || !permissionPopup?.matches(":popover-open")) return;
    const anchor = prompt.trigger.getBoundingClientRect();
    const viewport = scroller.getBoundingClientRect();
    if (anchor.bottom <= viewport.top || anchor.top >= viewport.bottom) {
      closePrompt(false);
      return;
    }
    const { width, height } = permissionPopup.getBoundingClientRect();
    permissionPopup.style.left = Math.max(12, Math.min(anchor.right - width, window.innerWidth - width - 12)) + "px";
    permissionPopup.style.top = Math.max(12, anchor.bottom + height + 20 <= window.innerHeight
      ? anchor.bottom + 8 : anchor.top - height - 8) + "px";
  }

  function closePrompt(restore = true) {
    const trigger = prompt?.trigger;
    if (permissionPopup?.matches(":popover-open")) permissionPopup.hidePopover();
    prompt = undefined;
    if (restore) trigger?.focus({ preventScroll: true });
  }

  function accept() {
    if (!prompt) return;
    const { addon, update } = prompt;
    const current = manager?.addons.find(item => item.id === addon.id);
    closePrompt();
    if (!current) return;
    if (addonAction(current) !== addonAction(addon) ||
        current.catalogPermissions.join() !== addon.catalogPermissions.join() ||
        (current.update ?? current.release)?.version !== (addon.update ?? addon.release)?.version) {
      errors[addon.id] = { key: "addons.store.releaseChanged", retry: () => {
        errors[addon.id] = undefined;
        const id = selected ? "addon-detail-action" : `addon-action-${addon.id}`;
        document.getElementById(id)?.click();
      } };
      return;
    }
    start(current, update);
  }

  function retryInstall(addon: ManagedAddon) {
    const id = selected ? "addon-detail-action" : `addon-action-${addon.id}`;
    document.getElementById(id)?.click();
  }
</script>

{#snippet icon(path: string)}
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={path} /></svg>
{/snippet}

{#snippet actionButton(addon: ManagedAddon, detail = false)}
  {@const action = addonAction(addon)}
  {#if action}
    {@const label = action === "cancel" ? $t("library.cancel") : $t(`addons.manager.${action}`)}
    {@const accessible = detail ? label : $t("addons.store.actionName", { action: label, name: addonText(addon.name, $locale) })}
    <button type="button" id={detail ? "addon-detail-action" : `addon-action-${addon.id}`}
      class:chrome={!detail} class:primary={detail && action !== "cancel"} class:secondary={detail && action === "cancel"}
      aria-label={accessible} title={accessible} aria-haspopup={action !== "cancel" && (action !== "update" ||
        addon.catalogPermissions.some(permission => !addon.permissions.includes(permission))) ? "dialog" : undefined}
      disabled={action === "cancel" ? cancelling[addon.id] : pending[addon.id]} onclick={event => void choose(event, addon)}>
      {#if detail}{label}{:else}{@render icon(icons[action === "reinstall" ? "install" : action])}{/if}
    </button>
  {/if}
{/snippet}

{#snippet errorLine(addon: ManagedAddon)}
  {@const error = errors[addon.id]}
  {#if error || addon.activity === "installFailed" || addon.activity === "updateIncompatible"}
    <div class="problem">
      <span role="alert">{error ? $t(error.key) : addon.problem ? $t(`addons.store.installProblem.${addon.problem}`) : $t(`addons.manager.activity.${addon.activity === "updateIncompatible" ? "updateIncompatible" : "installFailed"}`)}</span>
      {#if error || addonAction(addon)}
        <button type="button" class="quiet-action" onclick={error ? error.retry : () => retryInstall(addon)}>{$t("addons.store.retry")}</button>
      {/if}
    </div>
  {/if}
{/snippet}

<svelte:window onkeydown={escape} onresize={placePrompt} />

<main class="store" aria-labelledby="addons-title">
  <header>
    <div class="title">
      <button type="button" class="chrome" bind:this={initialFocus} aria-label={$t("addons.store.back")} title={$t("addons.store.back")} onclick={() => void back()}>
        {@render icon(icons.back)}
      </button>
      <h1 id="addons-title">{$t("addons.manager.title")}</h1>
    </div>
    <div class="tools">
      {#if !active}
        <input type="search" bind:value={query} aria-label={$t("addons.store.search")} placeholder={$t("addons.store.search")} />
      {/if}
      {#if dev && !active}
        <button class="quiet-action" type="button" disabled={sideloadPending} onclick={() => void loadLocalPack()}>
          {$t("contentPacks.loadLocal")}
        </button>
      {/if}
      {#if registryText}<span class="registry" role="status">{registryText}</span>{/if}
      <button type="button" class="chrome" aria-label={$t("addons.store.check")} title={$t("addons.store.check")}
        disabled={checking || registry?.checking} onclick={() => void check("manual")}>
        {@render icon(icons.update)}
      </button>
    </div>
  </header>

  <div class="scroll" bind:this={scroller} onscroll={placePrompt}>
    {#if sideloadFailed}
      <p class="page problem" role="alert">{$t("contentPacks.loadFailed")}</p>
    {/if}
    {#if listFailed}
      <div class="page problem"><span role="alert">{$t("addons.store.loadFailed")}</span>
        <button type="button" class="quiet-action" onclick={() => void refresh()}>{$t("addons.store.retry")}</button>
      </div>
    {/if}
    {#if active}
      {@const name = addonText(active.name, $locale)}
      {@const release = active.update ?? active.release}
      {@const action = addonAction(active)}
      {@const percent = downloadPercent(active.progress)}
      {@const adjustable = active.status === "enabled" || active.status === "disabled"}
      {@const locked = pending[active.id] || action === "cancel" || Boolean(active.approval) ||
        ["checking", "awaitingApproval", "importing", "publishing"].includes(active.activity)}
      <div class="page detail">
        <article>
          {#if activeShapes.length && !activeDetails?.preview}
            <div class="pack-previews">
              {#each activeShapes.slice(0, 3) as shape}
                <figure>
                  <div><PackShapePreview item={shape.item} /></div>
                  <figcaption>{shape.label}</figcaption>
                </figure>
              {/each}
            </div>
          {:else}
            <AddonPreview kind={active.kind ?? kinds[active.id]} image={activeDetails?.preview} large />
          {/if}
          {#if loading[active.id] || !activeDetails}
            <div class="skeleton" role="status" aria-label={$t("addons.store.loadingDescription")}><i></i><i></i><i></i></div>
          {:else if activeDetails.readme}
            <ReadmeView blocks={activeDetails.readme} />
          {:else}
            <p class="description-status">{$t(`addons.store.description.${activeDetails.problem ?? "unavailable"}`)}</p>
          {/if}
          {#if activeDetails?.reference && active.repository}
            <footer>
              <span>{$t("addons.store.source", { repository: active.repository, reference: activeDetails.reference })}</span>
              {#if activeDetails.verified}<span>{$t("addons.store.verified")}</span>{/if}
            </footer>
          {/if}
        </article>
        <aside aria-label={name}>
          <h2 tabindex="-1" bind:this={detailHeading}>{name}</h2>
          {#if summary(active, $locale)}<p class="muted">{summary(active, $locale)}</p>{/if}
          <div class="detail-actions">
            {#if action === "cancel"}
              <progress max="100" value={percent ?? undefined} aria-label={$t(active.activity === "installing" ? "addons.manager.activity.installing" : "addons.manager.activity.downloading")}></progress>
            {/if}
            {@render actionButton(active, true)}
          </div>
          <p class="status" role="status">
            {#if action === "cancel"}
              {$t(active.activity === "installing" ? "addons.manager.activity.installing" : "addons.manager.activity.downloading")}
              {#if percent !== null}{$t("addons.store.percent", { value: formatNumber(percent, undefined, $displayLocale) })}{/if}
            {:else}
              {$t(`addons.manager.status.${active.status}`, { expected: active.expectedApi, found: active.foundApi || $t("addons.manager.unknownVersion") })}
            {/if}
          </p>
          {@render errorLine(active)}
          <dl>
            {#if release}<dt>{$t("addons.store.version")}</dt><dd>{release.version}</dd>{/if}
            {#if active.installed}<dt>{$t("addons.manager.installed")}</dt><dd>{active.installed}</dd>{/if}
            {#if release}<dt>{$t("addons.store.size")}</dt><dd>{megabytes(release.sizeBytes)}</dd>
            {:else if active.status === "needs-reinstall"}<dt>{$t("addons.manager.download")}</dt><dd>{$t("addons.manager.noCompatibleRelease")}</dd>{/if}
            {#if active.formats.length}<dt>{$t("addons.manager.formats")}</dt><dd>{active.formats.join(", ")}</dd>{/if}
            {#if active.isolation !== "content"}
            <dt>{$t("addons.manager.limitsLabel")}</dt>
            <dd>{$t("addons.manager.limits", { memory: megabytes(active.memoryBytes), seconds: formatNumber(active.timeoutSeconds), output: megabytes(active.outputBytes) })}</dd>
            {/if}
          </dl>
          <section class="permissions">
            {#if active.isolation === "content"}
              <h3>{$t("contentPacks.contents")}</h3>
              {#if activePack}
                {#if activeShapes.length}
                  <p><strong>{$t("contentPacks.shapeCount", { count: activeShapes.length })}</strong>
                    {activeShapes.slice(0, 3).map(shape => shape.label).join(", ")}
                  </p>
                {/if}
                {#each [{ kind: "templates", items: activePack.templates }, { kind: "palettes", items: activePack.palettes }, { kind: "theme", items: activePack.themes }] as row}
                  {#if row.items.length}
                    <p><strong>{$t(`addons.store.kind.${row.kind as "templates" | "palettes" | "theme"}`)} ({formatNumber(row.items.length)})</strong>
                      {row.items.map(item => packText(activePack!, item.label, $locale)).join(", ")}
                    </p>
                  {/if}
                {/each}
                {#if activePack.typstPackages.length}
                  <p>{$t("contentPacks.typstPackages", { names: activePack.typstPackages.join(", ") })}</p>
                {/if}
              {:else}
                <p class="muted">{$t("contentPacks.unavailable")}</p>
              {/if}
            {:else if active.approval}
              {#key active.approval.token}
                <AddonApproval id={active.id} approval={active.approval} {refresh} />
              {/key}
            {:else}
              <AddonPermissions permissions={active.installed ? active.permissions : active.catalogPermissions} />
            {/if}
            <p class="muted">{$t(`addons.manager.isolation.${active.isolation}`)}</p>
            <p class="muted">{$t("addons.manager.network")}</p>
          </section>
          {#if adjustable}
            {@const contribution = contributions.find(entry => entry.addonId === active.id)}
            {#if contribution}
              {#key active.id + "/" + active.installed}
                <AddonSettings addon={contribution} disabled={locked || manager?.recovery} />
              {/key}
            {/if}
            <section class="switches">
              <button type="button" class="switch-row" role="switch" aria-checked={active.status === "enabled"} disabled={locked || manager?.recovery}
                onclick={() => void run(active, () => setAddonEnabled(active.id, active.status !== "enabled"))}>
                {$t("addons.manager.status.enabled")}<span></span>
              </button>
              {#if active.catalogPermissions.includes("add-page-objects")}
                <button type="button" class="switch-row" role="switch" aria-checked={active.directApply} disabled={locked || manager?.recovery}
                  onclick={() => void run(active, () => setAddonDirectApply(active.id, !active.directApply), "addons.manager.policyFailed")}>
                  {$t("addons.manager.direct")}<span></span>
                </button>
                <p class="status">{$t("addons.manager.directHint")}</p>
              {/if}
            </section>
          {/if}
          <div class="secondary-actions">
            {#if active.previous}
              <button type="button" class="secondary" disabled={locked} onclick={() => void run(active, () => rollbackAddon(active.id))}>
                {$t("addons.manager.rollback", { version: active.previous })}
              </button>
            {/if}
            {#if active.status !== "unsupported-os"}
              <button type="button" class="secondary" disabled={checking || registry?.checking} onclick={() => void check("manual")}>{$t("addons.manager.check")}</button>
            {/if}
            {#if active.installed}
              <button type="button" class="quiet-action remove" disabled={locked} onclick={() => confirming = true}>{$t("addons.manager.remove")}</button>
            {/if}
          </div>
          {#if confirming}
            <ConfirmDialog title={$t("addons.manager.confirmTitle", { name })} detail={$t("addons.manager.confirmDetail")}
              confirmLabel={$t("addons.manager.remove")} onCancel={() => confirming = false}
              onConfirm={() => { confirming = false; void run(active, () => removeAddon(active.id), "addons.manager.removeFailed"); }} />
          {/if}
        </aside>
      </div>
    {:else if manager}
      <div class="page grid">
        {#each visible as addon (addon.id)}
          {@const name = addonText(addon.name, $locale)}
          {@const kind = addon.kind ?? kinds[addon.id]}
          {@const kindLabel = kind === "import" ? $t("addons.store.kind.import") : kind === "recognize" ? $t("addons.store.kind.recognize") : kind === "theme" ? $t("addons.store.kind.theme") : addon.isolation === "content" ? $t(`addons.store.kind.${packKind(packs.find(pack => pack.id === addon.id))}`) : $t("addons.store.kind.other")}
          {@const release = addon.update ?? addon.release}
          {@const action = addonAction(addon)}
          {@const percent = downloadPercent(addon.progress)}
          <article class="tile">
            <AddonPreview {kind} image={details[addon.id]?.preview} {percent} busy={action === "cancel"} />
            <div class="tile-content">
              <button type="button" class="tile-open" bind:this={tileButtons[addon.id]} onclick={() => void openDetail(addon)}>{name}</button>
              <p class="tile-summary">{summary(addon, $locale)}</p>
              <div class="tile-bottom">
                <span class="meta">
                  {[kindLabel, release?.version ?? addon.installed, release && megabytes(release.sizeBytes),
                    percent !== null && action === "cancel" && $t("addons.store.percent", { value: formatNumber(percent, undefined, $displayLocale) })]
                    .filter(Boolean).join(" · ")}
                </span>
                {#if action}
                  {@render actionButton(addon)}
                {:else if addon.activity === "awaitingApproval"}
                  <span class="tile-status">{$t("addons.manager.activity.awaitingApproval")}</span>
                {:else if addon.status === "enabled"}
                  <span class="tile-status installed">{@render icon(icons.check)}{$t("addons.store.installed")}</span>
                {:else if addon.status !== "available"}
                  <span class="tile-status" class:warning={addon.status === "needs-reinstall" || addon.status === "incompatible-api"}>
                    {$t(`addons.manager.status.${addon.status}`, { expected: addon.expectedApi, found: addon.foundApi || $t("addons.manager.unknownVersion") })}
                  </span>
                {/if}
              </div>
              {@render errorLine(addon)}
            </div>
          </article>
        {:else}
          <p class="empty" role="status">{$t(query.trim() ? "addons.store.noResults" : "addons.store.empty")}</p>
        {/each}
      </div>
    {:else if !listFailed}
      <p class="page muted" role="status">{$t("addons.store.loading")}</p>
    {/if}
  </div>

  {#if prompt}
    {@const release = prompt.addon.update ?? prompt.addon.release}
    <div class="permission-popup" popover="auto" role="dialog" aria-labelledby="permission-title" bind:this={permissionPopup}
      ontoggle={event => { if (event.newState === "closed") prompt = undefined; }}>
      <h2 id="permission-title">{addonText(prompt.addon.name, $locale)}</h2>
      {#if release}<p class="muted">{megabytes(release.sizeBytes)}</p>{/if}
      <AddonPermissions permissions={newPermissions} update={prompt.update} install />
      <p class="muted">{$t(`addons.manager.isolation.${prompt.addon.isolation}`)}</p>
      <div class="popup-actions">
        <button type="button" class="secondary" onclick={() => closePrompt()}>{$t("library.cancel")}</button>
        <button type="button" class="primary" bind:this={confirmInstall} onclick={accept}>
          {$t(prompt.update ? "addons.manager.update" : "addons.manager.install")}
        </button>
      </div>
    </div>
  {/if}
</main>

<style>
  .store { position: fixed; inset: 0; z-index: 100; display: flex; flex-direction: column; background: var(--surround); color: var(--text); font: var(--text-md) var(--font-ui); }
  .store :global(::selection) { background: var(--accent-select); }
  header, .title, .tools { display: flex; align-items: center; gap: 12px; min-width: 0; }
  header { justify-content: space-between; padding: 12px 24px; border-bottom: 1px solid var(--edge-soft); }
  h1, h2 { margin: 0; font-size: 20px; font-weight: 600; }
  input { width: min(26vw, 280px); min-width: 145px; height: var(--control); padding: 0 10px; border: 1px solid var(--edge); border-radius: var(--radius); background: var(--inset); color: var(--text); font: inherit; caret-color: var(--blueprint-light); }
  input::placeholder { color: var(--quiet); }
  input:focus { border-color: var(--blueprint); }
  .registry, .status, footer, .tile-status { color: var(--muted); font-size: var(--text-sm); }
  .registry { text-align: end; }
  .scroll { flex: 1; overflow: auto; scrollbar-color: var(--edge-strong) var(--surround); }
  .page { width: min(100%, 1228px); margin: 0 auto; padding: 24px; box-sizing: border-box; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(280px, 100%), 1fr)); gap: 16px; align-items: start; }
  .empty { grid-column: 1 / -1; color: var(--muted); }
  .tile { position: relative; min-width: 0; padding: 8px; background: var(--panel); border: 1px solid var(--edge); border-radius: var(--radius-lg); }
  .tile:hover { background-color: var(--wash-strong); }
  .tile:hover .meta { color: var(--muted); }
  .tile-content { padding: 8px 4px 2px; }
  .tile-open { display: block; text-align: start; padding: 0; font-weight: 600; }
  .tile-open::after { content: ""; position: absolute; inset: 0; border-radius: var(--radius-lg); }
  .tile-open:focus-visible { outline: none; }
  .tile-open:focus-visible::after { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  .tile-summary { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; min-height: 2.8em; margin: 4px 0; color: var(--muted); line-height: 1.4; }
  .tile-bottom { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: var(--control-dense); }
  .meta { font-size: var(--text-sm); color: var(--quiet); font-variant-numeric: tabular-nums; }

  .tile-status { display: inline-flex; align-items: center; justify-content: end; gap: 4px; text-align: end; }
  .tile-status svg { width: 14px; height: 14px; stroke-width: var(--stroke-dense); flex: none; }
  .installed { color: var(--sage-text); }
  .warning { color: var(--amber-text); }
  .detail { max-width: 920px; display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: 32px; align-items: start; }
  .detail article { min-width: 0; }
  .detail article :global(.readme) { max-width: 72ch; }
  .detail aside { position: sticky; top: 24px; min-width: 0; }
  .muted, .description-status { color: var(--muted); line-height: 1.5; }
  .detail-actions, .secondary-actions, .popup-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .status { margin: 8px 0; font-variant-numeric: tabular-nums; }
  progress { flex: 1; min-width: 70px; height: 6px; accent-color: var(--blueprint); }
  dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 12px; margin: 14px 0 0; padding: 12px 0; border-block: 1px solid var(--edge-soft); }
  dt { color: var(--muted); }
  dd { margin: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
  .permissions h3 { font-size: var(--text-md); margin: 4px 0; }
  .pack-previews { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; margin-bottom: 14px; }
  .pack-previews figure { margin: 0; min-width: 0; overflow-wrap: anywhere; font-size: var(--text-sm); }
  .pack-previews figcaption { padding-top: 6px; color: var(--muted); }
  .pack-previews figure div { height: 94px; padding: 8px; background: var(--paper); border-radius: var(--radius); }
  .permissions, .switches { padding-block: 8px; border-bottom: 1px solid var(--edge-soft); }
  .secondary-actions { margin-top: 12px; }
  footer { display: grid; gap: 4px; margin-top: 16px; border-top: 1px solid var(--edge-soft); padding-top: 12px; overflow-wrap: anywhere; }
  .permission-popup { position: fixed; margin: 0; width: min(320px, calc(100vw - 24px)); max-height: calc(100dvh - 24px); overflow: auto; box-sizing: border-box; padding: 12px; border: 1px solid var(--edge); border-radius: var(--radius-lg); background: var(--panel); color: var(--text); box-shadow: var(--shadow-menu); }
  .permission-popup h2 { font-size: var(--text-lg); }
  .permission-popup p { margin: 6px 0; }
  .popup-actions { justify-content: end; margin-top: 12px; }
  .skeleton { display: grid; gap: 8px; margin: 20px 0; }
  .skeleton i { height: 10px; border-radius: var(--radius); background: var(--fill-soft); }
  .skeleton i:first-child { width: 35%; }
  .skeleton i:last-child { width: 75%; }
  button, .permissions :global(button) { border: 0; border-radius: var(--radius); background: transparent; color: inherit; font: inherit; cursor: pointer; }
  button:focus-visible, .permissions :global(button:focus-visible), input:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  button:disabled, .permissions :global(button:disabled) { opacity: var(--disabled); cursor: not-allowed; }
  .chrome { position: relative; flex: none; display: grid; place-items: center; width: var(--control); height: var(--control); padding: 0; color: var(--muted); }
  .tile .chrome { width: var(--control-dense); height: var(--control-dense); }
  .chrome svg { width: 20px; height: 20px; stroke-width: var(--stroke); }
  .tile .chrome svg { width: 16px; height: 16px; stroke-width: var(--stroke-dense); }
  .chrome:hover:not(:disabled), .quiet-action:hover:not(:disabled), .switch-row:hover:not(:disabled) { background: var(--wash); }
  .chrome:hover:not(:disabled) { color: var(--text); }
  .primary, .secondary, .permissions :global(button) { min-height: var(--control); padding: 0 14px; font-weight: 600; }
  .primary, .permissions :global(.primary) { background: var(--blueprint); color: var(--on-accent); }
  .primary:hover:not(:disabled), .permissions :global(.primary:hover:not(:disabled)) { background: var(--blueprint-hover); }
  .secondary, .permissions :global(.control) { background: var(--wash); }
  .secondary:hover:not(:disabled), .permissions :global(.control:hover:not(:disabled)) { background: var(--wash-strong); }
  .quiet-action { position: relative; padding: 4px 6px; color: inherit; text-decoration: underline; }
  .remove, .problem, .permissions :global(.problem) { color: var(--oxide-text); }
  .problem { position: relative; font-size: var(--text-sm); }
  .switch-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; text-align: start; width: 100%; min-height: var(--control); padding: 0 4px; }
  .switch-row span { flex: none; box-sizing: border-box; width: 34px; height: 20px; padding: 3px; border-radius: var(--radius-pill); background: var(--track); }
  .switch-row span::before { content: ""; display: block; width: 14px; height: 14px; border-radius: 50%; background: var(--muted); transition: transform var(--fast); }
  .switch-row[aria-checked="true"] span { background: var(--blueprint); }
  .switch-row[aria-checked="true"] span::before { background: var(--text); transform: translateX(14px); }
  @media (max-width: 899px) { .detail { grid-template-columns: minmax(0, 1fr); gap: 20px; } .detail aside { position: static; grid-row: 1; } }
  @media (max-width: 620px) { header { flex-wrap: wrap; } .tools { flex: 1; justify-content: space-between; } }
  @media (prefers-reduced-motion: reduce) { .switch-row span::before { transition: none; } }
</style>
