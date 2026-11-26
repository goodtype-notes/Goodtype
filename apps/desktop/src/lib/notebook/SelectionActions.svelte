<script lang="ts">
  import ShapeCapSelect from "./ShapeCapSelect.svelte";
  import type { ShapeCap } from "../model";
  import { tick } from "svelte";
  import { subjectLabel } from "../i18n/labels";
  import { t } from "../i18n";
  import OverflowMenu from "../workspace/OverflowMenu.svelte";
  import { populated, type MenuAction, type MenuSection } from "../workspace/menu";
  import { placeFloatingToolbar } from "../geometry/placement";
  import type { AlignMode, DistributeAxis } from "../geometry/align";

  /**
   * The bar beside a selection.
   *
   * Four actions stay in reach because they are what a selection is usually made for: ask the
   * assistant, copy, duplicate, remove. Paste joins them when the bar was opened by a long press
   * or right-click, because that press has already said where the paste goes. Everything else
   * sits behind one More button rather than widening the bar to twelve icons. A pen cannot
   * hover, so there is one level of menu and no flyouts.
   */
  type Props = {
    /** "page" is the context bar over empty canvas, where only Paste applies. */
    subject: "ink" | "image" | "shape" | "Typst block" | "items" | "page";
    left: number;
    top: number;
    ready: boolean;
    canMoveBack?: boolean;
    canMoveForward?: boolean;
    /** One layer at a time. Absent over empty canvas: there is nothing to move between layers. */
    onMove?: (direction: -1 | 1) => void;
    canArrangeBack?: boolean;
    canArrangeFront?: boolean;
    /** Straight to the back or the front. Objects only; ink moves a layer at a time. */
    onArrange?: (edge: "back" | "front") => void;
    canReadEarlier?: boolean;
    canReadLater?: boolean;
    /** Where screen readers announce the object. Objects only. */
    onReadingOrder?: (direction: -1 | 1) => void;
    onDelete?: () => void;
    grouped?: boolean;
    onGroup?: () => void;
    onConvert?: () => void;
    onShare?: () => void;
    onAI?: (anchor: { left: number; top: number }) => void;
    /** Ink only, and only where Windows offers a recognizer (ADR 0040). */
    onTranscribe?: () => void;
    /** Installed addons' ink-selection commands, labeled from their manifest strings. */
    addonCommands?: { id: string; label: string; icon?: string; onSelect: () => void }[];
    onCut?: () => void;
    onCopy?: () => void;
    /** Shown greyed when nothing has been copied, so the bar keeps its shape. */
    onPaste?: () => void;
    pasteDisabled?: boolean;
    onDuplicate?: () => void;
    /** Two or more objects: line them up or space them evenly. `count` is how many move as one. */
    align?: {
      count: number;
      onAlign: (mode: AlignMode) => void;
      onDistribute: (axis: DistributeAxis) => void;
    };
    shapeCaps?: {
      startCap?: ShapeCap;
      endCap?: ShapeCap;
      onChange: (end: "startCap" | "endCap", cap: ShapeCap) => void;
    };
    element?: HTMLElement;
  };

  let {
    subject,
    left,
    top,
    ready,
    canMoveBack = false,
    canMoveForward = false,
    onMove,
    canArrangeBack = false,
    canArrangeFront = false,
    onArrange,
    canReadEarlier = false,
    canReadLater = false,
    onReadingOrder,
    onDelete,
    grouped = false,
    onGroup,
    onConvert,
    onTranscribe,
    addonCommands = [],
    onShare,
    onAI,
    onCut,
    onCopy,
    onPaste,
    pasteDisabled = false,
    onDuplicate,
    align,
    shapeCaps,
    element = $bindable(),
  }: Props = $props();

  const what = $derived(subjectLabel(subject, $t));

  /** An entry only when its action exists, so callers keep deciding what applies by passing it. */
  function entry(
    id: string,
    label: string,
    icon: string,
    onSelect: (() => void) | undefined,
    extra: Partial<MenuAction> = {},
  ): MenuAction[] {
    return onSelect ? [{ kind: "action", id, label, icon, onSelect, ...extra }] : [];
  }

  const more = $derived<MenuSection[]>(populated([
    {
      entries: entry("cut", $t("selectionMenu.cut"),
        "M8.6 8.4 19 16M8.6 15.6 19 8M6.5 4.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5M6.5 14.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5",
        onCut, { hint: "Ctrl X" }),
    },
    {
      entries: [
        ...entry("back", $t("selectionMenu.backOneLayer"), "M15 19H6.5A1.5 1.5 0 0 1 5 17.5V9M8 5h11v11H8zM9 13l-3 3-3-3M6 16v-5",
          onMove && (() => onMove(-1)), { disabled: !canMoveBack }),
        ...entry("forward", $t("selectionMenu.forwardOneLayer"), "M9 19H6.5A1.5 1.5 0 0 1 5 17.5V9M8 5h11v11H8zM15 11l3-3 3 3M18 8v5",
          onMove && (() => onMove(1)), { disabled: !canMoveForward }),
        ...entry("to-back", $t("selectionMenu.sendToBack"), "M4 12l8 4 8-4M4 8l8-4 8 4-8 4z",
          onArrange && (() => onArrange("back")), { disabled: !canArrangeBack }),
        ...entry("to-front", $t("selectionMenu.bringToFront"), "M4 12l8-4 8 4-8 4zM4 16l8 4 8-4",
          onArrange && (() => onArrange("front")), { disabled: !canArrangeFront }),
      ],
    },
    {
      entries: [
        ...entry("read-earlier", $t("workspace.readEarlierByScreenReaders"), "M12 19V5m0 0-5 5m5-5 5 5",
          onReadingOrder && (() => onReadingOrder(-1)), { disabled: !canReadEarlier }),
        ...entry("read-later", $t("workspace.readLaterByScreenReaders"), "M12 5v14m0 0 5-5m-5 5-5-5",
          onReadingOrder && (() => onReadingOrder(1)), { disabled: !canReadLater }),
      ],
    },
    {
      entries: [
        ...entry("group", grouped ? $t("selectionMenu.ungroup") : $t("selectionMenu.groupWithTypst"),
          grouped
            ? "m8.5 15.5-2 2a3.2 3.2 0 0 1-4.5-4.5l3-3a3.2 3.2 0 0 1 4.5 0m6-1.5 2-2A3.2 3.2 0 0 1 22 11l-3 3a3.2 3.2 0 0 1-4.5 0M4 4l16 16"
            : "m9 15-2 2a3.2 3.2 0 0 1-4.5-4.5l3-3A3.2 3.2 0 0 1 10 14m5-5 2-2a3.2 3.2 0 0 1 4.5 4.5l-3 3A3.2 3.2 0 0 1 14 10m-6 6 8-8",
          onGroup),
        ...entry("convert", $t("selectionMenu.convertToShape"), "M4 17.5 9.5 5 14 16l3-8 3 9.5m-3-3.5 3 3-3 3", onConvert),
        ...entry("transcribe", $t("selectionMenu.transcribe"), "M3 16c1.5-5 3-5 4 0s2.5 5 4 0M14 6.5h7M17.5 6.5V18", onTranscribe),
        ...addonCommands.flatMap((command) => entry(`addon-${command.id}`, command.label,
          command.icon ?? "M9 4h6v3a2 2 0 1 0 4 0h1v6h-3a2 2 0 1 0 0 4h3v3H4V8h5z", command.onSelect)),
        ...entry("share", $t("helper.share"), "M4 4h16v12H9l-5 4z", onShare),
      ],
    },
  ]));

  const ALIGN_ICONS = {
    left: "M4 4v16M8 7h12M8 14h7",
    centre: "M12 4v16M6 8h12M8.5 15h7",
    right: "M20 4v16M4 7h12M9 14h7",
    top: "M4 4h16M7 8v12M14 8v7",
    middle: "M4 12h16M8 6v12M15 8.5v7",
    bottom: "M4 20h16M7 4v12M14 9v7",
    horizontal: "M4 4v16M20 4v16M9 8h6v8H9z",
    vertical: "M4 4h16M4 20h16M8 9h8v6H8z",
  };

  /** Align and Distribute as a menu of its own, so the bar keeps one button for eight actions. */
  const alignMenu = $derived<MenuSection[]>(align && align.count >= 2 ? populated([
    {
      entries: [
        ...entry("align-left", $t("selectionMenu.alignLeft"), ALIGN_ICONS.left, () => align.onAlign("left")),
        ...entry("align-centre", $t("selectionMenu.alignCentre"), ALIGN_ICONS.centre, () => align.onAlign("centre")),
        ...entry("align-right", $t("selectionMenu.alignRight"), ALIGN_ICONS.right, () => align.onAlign("right")),
        ...entry("align-top", $t("selectionMenu.alignTop"), ALIGN_ICONS.top, () => align.onAlign("top")),
        ...entry("align-middle", $t("selectionMenu.alignMiddle"), ALIGN_ICONS.middle, () => align.onAlign("middle")),
        ...entry("align-bottom", $t("selectionMenu.alignBottom"), ALIGN_ICONS.bottom, () => align.onAlign("bottom")),
      ],
    },
    {
      entries: [
        ...entry("distribute-horizontal", $t("selectionMenu.distributeHorizontally"), ALIGN_ICONS.horizontal,
          () => align.onDistribute("horizontal"), { disabled: align.count < 3 }),
        ...entry("distribute-vertical", $t("selectionMenu.distributeVertically"), ALIGN_ICONS.vertical,
          () => align.onDistribute("vertical"), { disabled: align.count < 3 }),
      ],
    },
  ]) : []);

  let openMenu = $state<"more" | "align" | null>(null);
  let moreButton = $state<HTMLButtonElement>();
  let alignButton = $state<HTMLButtonElement>();
  let popover = $state<HTMLElement>();

  async function toggleMenu(kind: "more" | "align") {
    if (openMenu === kind) { closeMenu(); return; }
    openMenu = kind;
    await tick();
    const button = kind === "align" ? alignButton : moreButton;
    const menu = popover?.querySelector<HTMLElement>(".overflow-menu");
    if (!popover || !menu || !button) return;
    // The top layer, so a bar near the edge of the page cannot clip its own menu.
    popover.showPopover();
    const placed = placeFloatingToolbar(button.getBoundingClientRect(), menu.getBoundingClientRect(),
      { left: 0, top: 0, right: innerWidth, bottom: innerHeight, width: innerWidth, height: innerHeight }, 6);
    popover.style.left = `${placed.left}px`;
    popover.style.top = `${placed.top}px`;
    menu.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
  }

  function closeMenu() {
    const restore = element?.contains(document.activeElement);
    const button = openMenu === "align" ? alignButton : moreButton;
    openMenu = null;
    if (restore) button?.focus();
  }
</script>

<div
  bind:this={element}
  class:ready
  class="selection-actions"
 
  role="toolbar"
  style:left={`${left}px`}
  style:top={`${top}px`}
  aria-label={subject === "page" ? $t("workspace.pageActions") : $t("workspace.selectedActions", { "subject": what })}
>
  {#if shapeCaps}
    <ShapeCapSelect end="start" value={shapeCaps.startCap} onChange={(cap) => shapeCaps?.onChange("startCap", cap)} />
    <ShapeCapSelect end="end" value={shapeCaps.endCap} onChange={(cap) => shapeCaps?.onChange("endCap", cap)} />
  {/if}
  {#if onAI}
    <button type="button" title={$t("aiEntry.shortcut")} aria-label={$t("aiEntry.ask")}
      onclick={event => { const rect = event.currentTarget.getBoundingClientRect(); onAI({ left: rect.left, top: rect.bottom }); }}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 5h14v10H10l-5 4Z" /></svg>
    </button>
  {/if}
  {#if onCopy}
    <button type="button" title={$t("workspace.copyCtrlC")} aria-label={$t("workspace.copySelected", { "subject": what })} onclick={onCopy}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="8.5" y="8.5" width="11" height="11" rx="2"></rect>
        <path d="M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"></path>
      </svg>
    </button>
  {/if}
  {#if onPaste}
    <button
      type="button"
      title={pasteDisabled ? $t("workspace.nothingCopiedYet") : $t("workspace.pasteHereCtrlV")}
      aria-label={$t("workspace.pasteHere")}
      disabled={pasteDisabled}
      onclick={onPaste}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="8.5" y="3.5" width="7" height="3.5" rx="1"></rect>
        <path d="M8.5 5.25H6.5a1.5 1.5 0 0 0-1.5 1.5v12.5a1.5 1.5 0 0 0 1.5 1.5h11a1.5 1.5 0 0 0 1.5-1.5V6.75a1.5 1.5 0 0 0-1.5-1.5h-2"></path>
      </svg>
    </button>
  {/if}
  {#if onDuplicate}
    <button type="button" title={$t("workspace.duplicateCtrlD")} aria-label={$t("workspace.duplicateSelected", { "subject": what })} onclick={onDuplicate}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="8.5" y="8.5" width="11" height="11" rx="2"></rect>
        <path d="M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"></path>
        <path d="M14 12v4M12 14h4"></path>
      </svg>
    </button>
  {/if}
  {#if onDelete}
    <button type="button" title={$t("workspace.remove2", { "subject": what })} aria-label={$t("workspace.removeSelectedFromPage", { subject: what })} onclick={onDelete}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4.5 7h15M9 7V4.5h6V7m2.5 0-.7 13h-9.6L6.5 7M10 10.5v6M14 10.5v6"></path>
      </svg>
    </button>
  {/if}
  {#if alignMenu.length}
    <button
      bind:this={alignButton}
      type="button"
      class:active={openMenu === "align"}
      title={$t("selectionMenu.align")}
      aria-label={$t("selectionMenu.align")}
      aria-haspopup="menu"
      aria-expanded={openMenu === "align"}
      onclick={() => void toggleMenu("align")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d={ALIGN_ICONS.left} /><path d="M8 18h9" /></svg>
    </button>
  {/if}
  {#if more.length}
    <button
      bind:this={moreButton}
      type="button"
      class:active={openMenu === "more"}
      title={$t("selectionMenu.more")}
      aria-label={$t("selectionMenu.more")}
      aria-haspopup="menu"
      aria-expanded={openMenu === "more"}
      onclick={() => void toggleMenu("more")}
    >
      <svg class="solid" viewBox="0 0 24 24" aria-hidden="true"><circle cx="5" cy="12" r="1.7"></circle><circle cx="12" cy="12" r="1.7"></circle><circle cx="19" cy="12" r="1.7"></circle></svg>
    </button>
  {/if}
  {#if openMenu}
    <div class="more-popover" popover="manual" bind:this={popover}>
      {#if openMenu === "align"}
        <OverflowMenu compact title={$t("selectionMenu.align")} sections={alignMenu} onClose={closeMenu} trigger={alignButton} />
      {:else}
        <OverflowMenu compact title={$t("selectionMenu.more")} sections={more} onClose={closeMenu} trigger={moreButton} />
      {/if}
    </div>
  {/if}
</div>

<style>
  .selection-actions {
    position: absolute;
    z-index: 25;
    display: flex;
    visibility: hidden;
    align-items: center;
    gap: 4px;
    padding: 4px;
    pointer-events: auto;
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    background: var(--panel);
    box-shadow: var(--shadow-menu);
  }
  .selection-actions.ready { visibility: visible; }
  button {
    display: grid;
    width: 40px;
    height: 40px;
    padding: 0;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    place-items: center;
    touch-action: manipulation;
    transition: background var(--fast), color var(--fast);
  }
  button:hover:not(:disabled), button.active { background: var(--wash); color: var(--text); }
  button:active:not(:disabled) { background: var(--wash-strong); }
  button:focus-visible { outline: 2px solid var(--blueprint-light); outline-offset: 2px; }
  button:disabled { opacity: var(--disabled); cursor: default; }
  svg {
    width: var(--icon);
    height: var(--icon);
    fill: none;
    stroke: currentColor;
    stroke-width: var(--stroke);
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  svg.solid { fill: currentColor; stroke: none; }
  .more-popover { position: fixed; inset: auto; margin: 0; padding: 0; border: 0; background: transparent; overflow: visible; }
  .more-popover :global(.overflow-menu) { position: relative; top: auto; right: auto; max-height: calc(100vh - 24px); overflow-y: auto; }
  @media (pointer: coarse) { button { width: var(--control-touch); height: var(--control-touch); } }
  @media (prefers-reduced-motion: reduce) { button { transition: none; } }
</style>
