<script lang="ts">
  import { errorDetails } from "../ipc/errors";
  import { t, notice, type UiText, displayLocale, formatDate } from "../i18n";
  import { onDestroy, untrack } from "svelte";
  import BrandMark from "../brand/BrandMark.svelte";
  import NamePrompt from "./NamePrompt.svelte";
  import NotebookSetup from "./NotebookSetup.svelte";
  import type { NotebookSetup as NotebookSetupValue } from "../page/presets";
  import NotebookCover from "./NotebookCover.svelte";
  import { importAddons, type ImportAddon } from "../ipc/addons";
  import ImportPanel from "./ImportPanel.svelte";
  import {
    allImporterStatus,
    importerBusy,
    type ImporterId,
    type ImporterStatus,
  } from "../ipc/importer";
  import ShelfMenu, { type ShelfMenuItem } from "./ShelfMenu.svelte";
  import type { ShelfLocation, ShelfView } from "./location";
  import {
    createLibraryFolder,
    createLibraryNotebook,
    deleteLibraryEntry,
    libraryFavourites,
    libraryRoot as readLibraryRoot,
    listLibrary,
    listLibraryFavourites,
    openLibraryNotebook,
    pickLibraryRoot,
    pickNotebookRoot,
    renameLibraryEntry,
    setLibraryFavourite,
  } from "../ipc/library";
  import {
    bands,
    breadcrumb,
    canMoveLibraryEntries,
    parentPath,
    type LibraryEntry,
    type LibraryListing,
    type SortOrder,
  } from "./library";

  let {
    tauriAvailable,
    coverRevision = 0,
    location = { view: "library", path: "" },
    onOpen,
    onCreate,
    onMove,
    returnLabel,
    onReturn,
    onLocationChange,
    onStatus,
    showNotebookSetup = false,
    onSettings,
    onAddons,
  }: {
    tauriAvailable: boolean;
    coverRevision?: number;
    /**
     * Where the shelf was left.
     *
     * Held by the caller rather than here, because this component is unmounted for as long as a
     * notebook is open — so anything it remembers itself is forgotten the moment you open
     * something, and closing would drop you back at the root. Coming out of a notebook should
     * put you where you went in from.
     */
    location?: ShelfLocation;
    /** Hands back an absolute notebook root, which is what every notebook command takes. */
    onOpen: (root: string) => void;
    /** Same, for a directory that is not a notebook yet and must be filled. */
    onCreate: (root: string, setup: NotebookSetupValue) => void;
    /** The workspace saves and remaps any open tabs around the Rust-owned filesystem move. */
    onMove: (paths: string[], destination: string) => Promise<string | null>;
    returnLabel?: string;
    onReturn?: () => void;
    onLocationChange?: (location: ShelfLocation) => void;
    onStatus: (message: UiText) => void;
    showNotebookSetup?: boolean;
    onAddons?: () => void;
    onSettings?: () => void;
  } = $props();

  const COVER_WIDTH_PX = 152;

  type Prompt =
    | { kind: "folder" }
    | { kind: "notebook" }
    | { kind: "rename"; path: string; initial: string };

  type MoveIntent = {
    lead: LibraryEntry;
    paths: string[];
  };

  type DragSession = MoveIntent & {
    pointerId: number;
    pointerType: string;
    startX: number;
    startY: number;
    x: number;
    y: number;
    active: boolean;
  };

  type DropTarget = {
    kind: "folder" | "crumb";
    path: string;
  };

  let libraryRoot = $state<string | null>(null);
  // Seeded once from the caller and reported back on every move, so the two never fight: this
  // owns it while the shelf is on screen, the caller holds it while a notebook is.
  let view = $state<ShelfView>(untrack(() => location.view));
  let path = $state(untrack(() => location.path));
  let entries = $state.raw<LibraryEntry[]>([]);
  let favourites = $state.raw<string[]>([]);
  let order = $state<SortOrder>("name");
  let busy = $state(false);
  let failure = $state<string | null>(null);
  let mutationFailure = $state<string | null>(null);
  let setupShown = false;

  let menu = $state<"new" | "sort" | null>(null);
  let entryMenu = $state<string | null>(null);
  let prompt = $state<Prompt | null>(null);
  /// The import panel. Jobs run in Rust, so it reopens on its own when the
  /// Library comes back while an import is running or has a result waiting.
  let importing = $state(false);
  /// The source the panel opens on: the one with a job, else the one used last this session.
  let importTool = $state<ImporterId>("");
  let importSources = $state.raw<ImportAddon[]>([]);
  /// While the panel is closed: a job still running, or a finished one with a result to read.
  let importJob = $state<"running" | "done" | null>(null);
  let importPoll: ReturnType<typeof setInterval> | undefined;

  /** The source whose job needs attention: running first, then a result waiting to be read. */
  function sourceWithJob(statuses: Record<ImporterId, ImporterStatus>): ImporterId | null {
    const ids = Object.keys(statuses);
    const kind = (id: ImporterId) => statuses[id].activity.kind;
    return (
      ids.find((id) => importerBusy(statuses[id].activity)) ??
      ids.find((id) => kind(id) === "finished" || kind(id) === "importFailed") ??
      ids.find((id) => kind(id) !== "idle") ??
      null
    );
  }

  function closeImport() {
    importing = false;
    void followImport();
  }

  async function openImport() {
    stopImportPoll();
    try {
      importSources = await importAddons();
    } catch (error) {
      failure = message(error);
      return;
    }
    try {
      const statuses = await allImporterStatus();
      importSources = importSources.filter(({ id }) => !statuses[id]?.disabled);
      importTool = sourceWithJob(statuses) ?? importTool;
    } catch {
      // The panel reports a status it cannot read itself.
    }
    const first = importSources[0];
    if (!first) return;
    if (!importSources.some(({ id }) => id === importTool)) importTool = first.id;
    importJob = null;
    importing = true;
  }

  function stopImportPoll() {
    if (importPoll) clearInterval(importPoll);
    importPoll = undefined;
  }

  /// Keeps the toolbar button honest while the panel is closed, and refreshes the shelf the moment
  /// new notebooks have arrived.
  async function followImport() {
    stopImportPoll();
    const check = async () => {
      try {
        const statuses = await allImporterStatus();
        if (Object.values(statuses).some(({ activity }) => importerBusy(activity))) {
          importJob = "running";
          return;
        }
        stopImportPoll();
        const kinds = Object.values(statuses).map(({ activity }) => activity.kind);
        if (kinds.includes("finished") || kinds.includes("importFailed")) {
          if (importJob === "running") void reload();
          importJob = "done";
        } else {
          importJob = null;
        }
      } catch {
        stopImportPoll();
        importJob = null;
      }
    };
    await check();
    if (importJob === "running") importPoll = setInterval(() => void check(), 1500);
  }
  /** Null means plain browsing; a set — even an empty one — means select mode is on. */
  let picked = $state.raw<string[] | null>(null);
  let moving = $state.raw<MoveIntent | null>(null);
  let drag = $state<DragSession | null>(null);
  let dropTarget = $state<DropTarget | null>(null);
  let contentsElement: HTMLDivElement | undefined;
  let queuedDragPoint: { x: number; y: number } | null = null;
  let dragFrame: number | null = null;
  let suppressClickPath: string | null = null;

  const crumbs = $derived(breadcrumb(path, $t("library.library")));
  const shelf = $derived(bands(entries, order));
  const up = $derived(parentPath(path));
  const starred = $derived(new Set(favourites));
  const selecting = $derived(picked !== null);

  $effect(() => {
    void start();
  });

  onDestroy(() => {
    listingGeneration += 1;
    clearDrag();
    stopImportPoll();
  });

  async function start() {
    if (!tauriAvailable) return;
    try {
      libraryRoot = await readLibraryRoot();
      if (libraryRoot) {
        await reload();
        void allImporterStatus()
          .then(async (statuses) => {
            const source = sourceWithJob(statuses);
            if (source) {
              importSources = (await importAddons()).filter(({ id }) => !statuses[id]?.disabled);
              importTool = source;
              importing = true;
            }
          })
          .catch(() => {});
        if (showNotebookSetup && !setupShown) {
          setupShown = true;
          prompt = { kind: "notebook" };
        }
      }
    } catch (error) {
      failure = message(error);
    }
  }

  async function chooseLibrary() {
    busy = true;
    try {
      const chosen = await pickLibraryRoot();
      if (!chosen) return;
      libraryRoot = chosen;
      view = "library";
      path = "";
      await reload();
      if (showNotebookSetup && !setupShown) {
        setupShown = true;
        prompt = { kind: "notebook" };
      }
      onStatus(notice("library.librarySetTo", { "chosen": chosen }));
    } catch (error) {
      failure = message(error);
    } finally {
      busy = false;
    }
  }

  /**
   * Read whatever the current view is showing.
   *
   * A folder listing echoes the path it was asked for, and anything answering for a folder the
   * writer has already navigated away from is dropped. Without that, a slow listing of a deep
   * folder can land after a fast one of its parent and repaint the wrong shelf.
   */
  let listingGeneration = 0;
  async function reload() {
    const generation = ++listingGeneration;
    const requestedRoot = libraryRoot;
    const requestedView = view;
    const wanted = path;
    const current = () => generation === listingGeneration && libraryRoot === requestedRoot &&
      view === requestedView && path === wanted;
    busy = true;
    failure = null;
    try {
      const stars = await libraryFavourites();
      if (!current()) return;
      if (requestedView === "favourites") {
        const result = await listLibraryFavourites();
        if (!current()) return;
        entries = result;
      } else {
        const listing = await listLibrary(wanted);
        if (!current() || listing.path !== wanted) return;
        entries = listing.entries;
      }
      favourites = stars;
    } catch (error) {
      if (!current()) return;
      entries = [];
      failure = message(error);
    } finally {
      if (current()) busy = false;
    }
  }

  async function show(next: string) {
    clearDrag();
    moving = null;
    view = "library";
    path = next;
    picked = null;
    onLocationChange?.({ view, path });
    await reload();
  }

  async function showFavourites() {
    clearDrag();
    moving = null;
    view = "favourites";
    picked = null;
    onLocationChange?.({ view, path });
    await reload();
  }

  async function openNotebook(notebookPath: string) {
    busy = true;
    try {
      onOpen(await openLibraryNotebook(notebookPath));
    } catch (error) {
      failure = message(error);
      busy = false;
    }
  }

  /** Run a change to the library, then re-read rather than patching the list in place. */
  async function mutate(work: () => Promise<unknown>, done?: string): Promise<boolean> {
    busy = true;
    mutationFailure = null;
    try {
      await work();
      if (done) onStatus(done);
      await reload();
      return true;
    } catch (error) {
      mutationFailure = message(error);
      busy = false;
      return false;
    }
  }

  async function confirmPrompt(name: string) {
    const pending = prompt;
    prompt = null;
    if (!pending) return;
    if (pending.kind === "folder") {
      await mutate(
        () => createLibraryFolder(path, name),
        $t("library.createdFolder", { "name": name }),
      );
    } else if (pending.kind === "rename") {
      await mutate(
        () => renameLibraryEntry(pending.path, name),
        $t("library.renamedTo", { "name": name }),
      );
    }
  }

  async function createNotebook(setup: NotebookSetupValue) {
    prompt = null;
    busy = true;
    try {
      const root = await createLibraryNotebook(path, setup.name);
      onCreate(root, setup);
    } catch (error) {
      failure = message(error);
      busy = false;
    }
  }

  async function toggleFavourite(entryPath: string) {
    await mutate(() =>
      setLibraryFavourite(entryPath, !starred.has(entryPath)),
    );
  }

  async function remove(paths: string[]) {
    await mutate(async () => {
      for (const each of paths) await deleteLibraryEntry(each);
    }, paths.length === 1 ? $t("library.movedToTheTrash") : $t("library.movedItemsToTheTrash", { "length": paths.length }));
    picked = null;
  }

  /** Move one tile or a whole selection onto a folder tile or crumb. */
  async function moveTo(sourcePaths: string[], destination: string) {
    clearDrag();
    dropTarget = null;
    moving = null;
    if (!canMoveLibraryEntries(sourcePaths, destination)) return;
    picked = null;
    let moveStatus: string | null = null;
    const moved = await mutate(async () => {
      moveStatus = await onMove(sourcePaths, destination);
    });
    if (moved) {
      onStatus(
        moveStatus ?? (sourcePaths.length === 1
          ? $t("library.moved", { "value1": pathName(sourcePaths[0]) })
          : $t("library.movedItems", { "length": sourcePaths.length })),
      );
    }
  }

  function beginMove(lead: LibraryEntry, sourcePaths = [lead.path]) {
    moving = { lead, paths: [...sourcePaths] };
    entryMenu = null;
    menu = null;
  }

  function beginPickedMove() {
    const paths = picked ?? [];
    const lead = entries.find((entry) => paths.includes(entry.path));
    if (lead && paths.length > 0) beginMove(lead, paths);
  }

  function chooseDestination(destination: string) {
    if (moving && canMoveLibraryEntries(moving.paths, destination)) {
      void moveTo(moving.paths, destination);
    }
  }

  function dragPaths(entry: LibraryEntry): string[] {
    if (!selecting) return [entry.path];
    return picked?.includes(entry.path) ? [...picked] : [];
  }

  function beginEntryDrag(event: PointerEvent, entry: LibraryEntry) {
    if (
      busy ||
      moving ||
      !event.isPrimary ||
      event.button !== 0 ||
      event.pointerType === "touch"
    ) {
      return;
    }
    const paths = dragPaths(entry);
    if (paths.length === 0) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    drag = {
      lead: entry,
      paths,
      pointerId: event.pointerId,
      pointerType: event.pointerType,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      active: false,
    };
  }

  function continueEntryDrag(event: PointerEvent) {
    const session = drag;
    if (!session || session.pointerId !== event.pointerId) return;
    if (!session.active) {
      const threshold = session.pointerType === "pen" ? 4 : 6;
      if (Math.hypot(event.clientX - session.startX, event.clientY - session.startY) < threshold) {
        return;
      }
      entryMenu = null;
      menu = null;
      drag = { ...session, active: true, x: event.clientX, y: event.clientY };
    }
    event.preventDefault();
    queueDrag(event.clientX, event.clientY);
  }

  function finishEntryDrag(event: PointerEvent) {
    const session = drag;
    if (!session || session.pointerId !== event.pointerId) return;
    // Resolve once more at release: a fast pen can enter a target and lift before the queued
    // animation frame has painted its hover state.
    const destination = targetAt(session.paths, event.clientX, event.clientY)?.path;
    const wasActive = session.active;
    clearDrag();
    const target = event.currentTarget as HTMLElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    if (!wasActive) return;

    event.preventDefault();
    event.stopPropagation();
    suppressClickPath = session.lead.path;
    window.setTimeout(() => {
      if (suppressClickPath === session.lead.path) suppressClickPath = null;
    }, 0);
    if (destination !== undefined) void moveTo(session.paths, destination);
  }

  function cancelEntryDrag(event?: PointerEvent) {
    if (!event || drag?.pointerId === event.pointerId) clearDrag();
  }

  function clearDrag() {
    if (dragFrame !== null && typeof cancelAnimationFrame !== "undefined") {
      cancelAnimationFrame(dragFrame);
    }
    dragFrame = null;
    queuedDragPoint = null;
    drag = null;
    dropTarget = null;
  }

  function queueDrag(x: number, y: number) {
    queuedDragPoint = { x, y };
    if (dragFrame !== null) return;
    dragFrame = requestAnimationFrame(updateDragFrame);
  }

  function updateDragFrame() {
    dragFrame = null;
    const point = queuedDragPoint;
    queuedDragPoint = null;
    const session = drag;
    if (!point || !session?.active) return;

    drag = { ...session, x: point.x, y: point.y };
    dropTarget = targetAt(session.paths, point.x, point.y);
    if (autoScroll(point.y)) queueDrag(point.x, point.y);
  }

  function targetAt(sourcePaths: string[], x: number, y: number): DropTarget | null {
    const target = document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>("[data-library-drop-path]");
    const destination = target?.dataset.libraryDropPath;
    const kind = target?.dataset.libraryDropKind;
    if (
      destination === undefined ||
      (kind !== "folder" && kind !== "crumb") ||
      !canMoveLibraryEntries(sourcePaths, destination)
    ) {
      return null;
    }
    return { kind, path: destination };
  }

  function autoScroll(pointerY: number): boolean {
    const element = contentsElement;
    if (!element) return false;
    const rect = element.getBoundingClientRect();
    const edge = 68;
    let delta = 0;
    if (pointerY < rect.top + edge) {
      delta = -12 * Math.min(1, (rect.top + edge - pointerY) / edge);
    } else if (pointerY > rect.bottom - edge) {
      delta = 12 * Math.min(1, (pointerY - (rect.bottom - edge)) / edge);
    }
    if (delta === 0) return false;
    const before = element.scrollTop;
    element.scrollTop += delta;
    return element.scrollTop !== before;
  }

  function isPointerTarget(kind: DropTarget["kind"], destination: string): boolean {
    return dropTarget?.kind === kind && dropTarget.path === destination;
  }

  function isMoveTarget(destination: string): boolean {
    return moving !== null && canMoveLibraryEntries(moving.paths, destination);
  }

  function handleWindowKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape" || (!drag && !moving)) return;
    event.preventDefault();
    clearDrag();
    moving = null;
  }

  function dragPreviewStyle(session: DragSession): string {
    const left = Math.min(session.x + 16, window.innerWidth - 236);
    const top = Math.min(session.y + 16, window.innerHeight - 54);
    return `transform: translate3d(${Math.max(8, left)}px, ${Math.max(8, top)}px, 0)`;
  }

  function pathName(entryPath: string): string {
    return entryPath.split("/").at(-1) ?? entryPath;
  }

  /** The library folder's own name, from an OS path in either separator style. */
  function folderName(root: string): string {
    return root.replace(/[\\/]+$/, "").split(/[\\/]/).at(-1) || root;
  }

  function togglePicked(entryPath: string) {
    const current = picked ?? [];
    picked = current.includes(entryPath)
      ? current.filter((each) => each !== entryPath)
      : [...current, entryPath];
  }

  function activate(entry: LibraryEntry) {
    if (suppressClickPath === entry.path) return;
    if (moving) {
      if (entry.kind === "folder") chooseDestination(entry.path);
      return;
    }
    if (selecting) {
      togglePicked(entry.path);
      return;
    }
    if (entry.kind === "folder") void show(entry.path);
    else void openNotebook(entry.path);
  }

  function entryMenuItems(entry: LibraryEntry): ShelfMenuItem[] {
    return [
      {
        id: "rename",
        label: $t("library.rename"),
        onSelect: () => (prompt = { kind: "rename", path: entry.path, initial: entry.name }),
      },
      {
        id: "favourite",
        label: starred.has(entry.path) ? $t("library.removeFromFavourites") : $t("library.addToFavourites"),
        onSelect: () => void toggleFavourite(entry.path),
      },
      {
        id: "move",
        label: $t("library.move"),
        onSelect: () => beginMove(entry),
      },
      {
        id: "delete",
        label: $t("library.moveToTrash"),
        destructive: true,
        onSelect: () => void remove([entry.path]),
      },
    ];
  }

  const sortItems: ShelfMenuItem[] = $derived([
    {
      id: "name",
      label: $t("library.name"),
      checked: order === "name",
      onSelect: () => (order = "name"),
    },
    {
      id: "modified",
      label: $t("library.lastModified"),
      checked: order === "modified",
      onSelect: () => (order = "modified"),
    },
  ]);

  const newItems: ShelfMenuItem[] = $derived([
    { id: "notebook", label: $t("library.newNotebook"), onSelect: () => (prompt = { kind: "notebook" }) },
    { id: "folder", label: $t("library.newFolder"), onSelect: () => (prompt = { kind: "folder" }) },
    { id: "import", label: $t("library.importNotebooks2"), onSelect: () => void openImport() },
    // The way out of the library, kept because a notebook someone sends you lands in Downloads
    // and has to be openable without being filed first.
    { id: "elsewhere", label: $t("library.openAnotherNotebook"), onSelect: () => void openElsewhere() },
  ]);

  async function openElsewhere() {
    busy = true;
    try {
      const chosen = await pickNotebookRoot();
      if (chosen) onOpen(chosen);
      else busy = false;
    } catch (error) {
      failure = message(error);
      busy = false;
    }
  }

  function message(error: unknown): string {
    return errorDetails(error);
  }

  function whenModified(modifiedMs: number | null): string {
    if (!modifiedMs) return "—";
    return formatDate(modifiedMs, $displayLocale);
  }

  function pages(count: number | null): string {
    if (count === null) return $t("library.manifestUnlesbar");
    return $t("counts.pages", { count });
  }

  function items(count: number): string {
    return $t("counts.items", { count });
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} onblur={() => clearDrag()} />

<div class="library" class:dragging={drag?.active}>
  <aside class="rail">
    <div class="brand">
      <BrandMark size={20} title="" />
      <span>goodtype</span>
    </div>

    {#if onReturn}
      <button type="button" class="return-to-notebook" onclick={onReturn}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
        <span>{returnLabel ?? $t("library.openNotebook")}</span>
      </button>
    {/if}

    {#if libraryRoot}
      <nav class="views">
        <button
          type="button"
          class="view"
          class:current={view === "library"}
          aria-current={view === "library" ? "page" : undefined}
          onclick={() => void show("")}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 5.5h6.5v13H4zM13.5 5.5H20v13h-6.5z" />
            <path d="M7.25 9h0M16.75 9h0" />
          </svg>
          <span class="grow">{$t("library.library")}</span>
        </button>
        <button
          type="button"
          class="view"
          class:current={view === "favourites"}
          aria-current={view === "favourites" ? "page" : undefined}
          onclick={() => void showFavourites()}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m12 4 2.4 5 5.6.7-4 3.9 1 5.4-5-2.7-5 2.7 1-5.4-4-3.9 5.6-.7L12 4Z" />
          </svg>
          <span class="grow">{$t("library.favourites")}</span>
        </button>
      </nav>
    {/if}

    <!-- The design put a folder tree here. It is deliberately absent: at the root it lists
         exactly what the grid already shows, it needs expand state and a cached listing per node
         that every rename and move has to invalidate, and it is the one control on this surface
         you would have to put the pen down for. Jumping between branches is what Favoriten and
         search are for, and moving is a drag onto a crumb or a folder tile. -->

    {#if onSettings || onAddons || libraryRoot}
      <div class="rail-footer">
        {#if libraryRoot}
          <!-- Where the shelf reads from: the folder's own name, with the full path on hover. Rarely
               changed, so it sits with the other rarely used things rather than above the views. -->
          <div class="where" title={libraryRoot}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2.5h8A1.5 1.5 0 0 1 20.5 10v8A1.5 1.5 0 0 1 19 19.5H5A1.5 1.5 0 0 1 3.5 18V7.5Z" />
            </svg>
            <span class="where-name">{folderName(libraryRoot)}</span>
            <button
              type="button"
              class="link"
              aria-label={$t("library.changeLibraryFolder")}
              title={$t("library.changeLibraryFolder")}
              disabled={busy}
              onclick={chooseLibrary}
            >{$t("library.change")}</button>
          </div>
        {/if}
        {#if onAddons}
          <button type="button" class="settings-link" onclick={onAddons}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m4 7 8-4 8 4v10l-8 4-8-4zM4 7l8 4 8-4m-8 4v10m-4-16 8 4" /></svg>
            <span>{$t("addons.manager.title")}</span>
          </button>
        {/if}
        {#if onSettings}
          <button type="button" class="settings-link" aria-haspopup="dialog" onclick={onSettings}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M10 3h4l.5 2.5 1.7 1 2.4-.8 2 3.5-1.9 1.7v2.2l1.9 1.7-2 3.5-2.4-.8-1.7 1L14 21h-4l-.5-2.5-1.7-1-2.4.8-2-3.5 1.9-1.7v-2.2L3.4 9.2l2-3.5 2.4.8 1.7-1z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
            <span>{$t("library.settings")}</span>
          </button>
        {/if}
      </div>
    {/if}
  </aside>

  <section class="shelf" aria-busy={busy}>
    <header class="bar">
      {#if view === "favourites"}
        <span class="here-label">{$t("library.favourites")}</span>
      {:else}
        <nav class="crumbs" aria-label={$t("library.path")}>
          {#each crumbs as crumb, index (crumb.path)}
            {#if index > 0}<span class="separator" aria-hidden="true">/</span>{/if}
            <button
              type="button"
              class="crumb"
              class:here={index === crumbs.length - 1}
              class:drop={isPointerTarget("crumb", crumb.path)}
              class:moveTarget={isMoveTarget(crumb.path)}
              data-library-drop-path={crumb.path}
              data-library-drop-kind="crumb"
              aria-current={index === crumbs.length - 1 ? "page" : undefined}
              aria-label={moving && isMoveTarget(crumb.path)
                ? $t("library.moveTheSelectionHere", { "name": crumb.name })
                : crumb.name}
              disabled={busy || (moving !== null && !isMoveTarget(crumb.path))}
              onclick={() => moving ? chooseDestination(crumb.path) : void show(crumb.path)}
              >{crumb.name}</button
            >
          {/each}
        </nav>
      {/if}

      <div class="spacer"></div>

      {#if libraryRoot}
        {#if importJob && !importing}
          <button type="button" class="control import-job" class:done={importJob === "done"} onclick={() => void openImport()}>
            {#if importJob === "running"}
              <span class="import-pulse" aria-hidden="true"></span>
              <span>{$t("library.importRunning3")}</span>
            {:else}
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12.5 4.5 4.5L19 7.5" /></svg>
              <span>{$t("library.importFinished")}</span>
            {/if}
          </button>
        {/if}
        <div class="anchor">
          <button
            type="button"
            class="control menu-control"
            aria-haspopup="menu"
            aria-expanded={menu === "sort"}
            disabled={busy || moving !== null}
            onclick={() => (menu = menu === "sort" ? null : "sort")}
          >
            <span>{$t("library.sort", { "value2": order === "name" ? $t("library.name") : $t("library.date") })}</span>
            <svg class:open={menu === "sort"} viewBox="0 0 24 24" aria-hidden="true">
              <path d="m7 9.5 5 5 5-5" />
            </svg>
          </button>
          {#if menu === "sort"}
            <ShelfMenu label={$t("library.sort2")} items={sortItems} onClose={() => (menu = null)} />
          {/if}
        </div>

        <button
          type="button"
          class="control"
          aria-pressed={selecting}
          disabled={busy || moving !== null}
          onclick={() => (picked = selecting ? null : [])}
        >
          {selecting ? $t("library.done") : $t("library.select")}
        </button>

        {#if view === "library"}
          <div class="anchor">
            <button
              type="button"
              class="primary menu-control"
              aria-haspopup="menu"
              aria-expanded={menu === "new"}
              disabled={busy || moving !== null}
              onclick={() => (menu = menu === "new" ? null : "new")}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
              <span>{$t("library.new")}</span>
              <svg class="menu-caret" class:open={menu === "new"} viewBox="0 0 24 24" aria-hidden="true">
                <path d="m7 9.5 5 5 5-5" />
              </svg>
            </button>
            {#if menu === "new"}
              <ShelfMenu label={$t("library.new")} items={newItems} onClose={() => (menu = null)} />
            {/if}
          </div>
        {/if}
      {/if}
    </header>

    {#if moving}
      <div class="action-bar move-bar" role="status">
        <span class="move-title">
          {$t("library.move2", { "value2": moving.paths.length === 1 ? `“${moving.lead.name}”` : $t("library.items2", { "length": moving.paths.length }) })}
        </span>
        <span class="move-help">{$t("library.chooseADestinationFolderOrPath")}</span>
        <div class="spacer"></div>
        <button type="button" class="control" onclick={() => (moving = null)}>{$t("library.cancel")}</button>
      </div>
    {:else if selecting && picked && picked.length > 0}
      <div class="action-bar">
        <span>{$t("library.selected", { "length": picked.length })}</span>
        <div class="spacer"></div>
        <button type="button" class="control" disabled={busy} onclick={beginPickedMove}>
          {$t("library.move")}
        </button>
        <button type="button" class="control destructive" disabled={busy} onclick={() => void remove(picked ?? [])}>
          {$t("library.moveToTrash")}
        </button>
      </div>
    {/if}

    {#if mutationFailure}
      <div class="operation-error" role="alert">
        <span>{$t("errors.operation", { details: mutationFailure })}</span>
        <div class="spacer"></div>
        <button
          type="button"
          class="error-dismiss"
          aria-label={$t("library.dismissError")}
          onclick={() => (mutationFailure = null)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
        </button>
      </div>
    {/if}

    <div class="contents" bind:this={contentsElement}>
      {#if !tauriAvailable}
        <p class="notice">{$t("library.theLibraryNeedsTheDesktopAppTo")}</p>
      {:else if !libraryRoot}
        <div class="first-run">
          <svg class="first-run-mark" viewBox="0 0 24 24" aria-hidden="true">
              <path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2.5h8A1.5 1.5 0 0 1 20.5 10v8A1.5 1.5 0 0 1 19 19.5H5A1.5 1.5 0 0 1 3.5 18V7.5Z" />
          </svg>
          <h1>{$t("library.whereShouldYourNotebooksLive")}</h1>
          <p>
            {$t("library.chooseTheFolderGoodtypeShowsYourNotebooks")}
          </p>
          <button type="button" class="primary" disabled={busy} onclick={chooseLibrary}>
            {$t("library.chooseLibraryFolder")}
          </button>
        </div>
      {:else if failure}
        <p class="notice failure">{$t("errors.operation", { details: failure })}</p>
      {:else if entries.length === 0}
        <div class="empty">
          <p>
            {busy
              ? $t("library.loading")
              : view === "favourites"
                ? $t("library.noFavouritesYet")
                : $t("library.thisFolderIsEmpty")}
          </p>
          {#if view === "library" && up !== null && !busy}
            <button type="button" class="control" onclick={() => void show(up)}>
              {$t("library.upOneLevel")}
            </button>
          {/if}
        </div>
      {:else}
        {#if shelf.folders.length > 0}
          <h2 class="band">{$t("library.folders")}</h2>
          <div class="grid folders">
            {#each shelf.folders as folder (folder.path)}
              <div
                class="tile folder"
                class:chosen={picked?.includes(folder.path)}
                class:drop={isPointerTarget("folder", folder.path)}
                class:moveTarget={isMoveTarget(folder.path)}
                class:draggingSource={drag?.active && drag.paths.includes(folder.path)}
                data-library-drop-path={folder.path}
                data-library-drop-kind="folder"
                role="presentation"
              >
                <button
                  type="button"
                  class="hit"
                  aria-label={moving && isMoveTarget(folder.path)
                    ? $t("library.moveTheSelectionHere", { "name": folder.name })
                    : folder.name}
                  aria-disabled={moving !== null && !isMoveTarget(folder.path)}
                  aria-pressed={selecting ? picked?.includes(folder.path) : undefined}
                  disabled={busy}
                  onpointerdown={(event) => beginEntryDrag(event, folder)}
                  onpointermove={continueEntryDrag}
                  onpointerup={finishEntryDrag}
                  onpointercancel={cancelEntryDrag}
                  onlostpointercapture={cancelEntryDrag}
                  onclick={() => activate(folder)}
                >
                  <svg width="19" height="19" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                    <path
                      d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2.5h7A1.5 1.5 0 0 1 19 10v7.5A1.5 1.5 0 0 1 17.5 19h-13A1.5 1.5 0 0 1 3 17.5v-10z"
                      stroke="currentColor"
                      stroke-width="1.5"
                    />
                  </svg>
                  <span class="tile-text">
                    <span class="tile-name">{#if starred.has(folder.path)}<svg class="star" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4l2.4 5 5.6.7-4 3.9 1 5.4-5-2.7-5 2.7 1-5.4-4-3.9 5.6-.7L12 4z" /></svg>{/if}<span class="name-text">{folder.name}</span></span>
                    <span class="tile-meta">
                      {isPointerTarget("folder", folder.path) ? $t("library.dropHere") : items(folder.childCount)}
                    </span>
                  </span>
                </button>
                {@render tileMarks(folder)}
              </div>
            {/each}
          </div>
        {/if}

        {#if shelf.notebooks.length > 0}
          <h2 class="band">{$t("library.notebooks")}</h2>
          <div class="grid">
            {#each shelf.notebooks as notebook (notebook.path)}
              <div
                class="tile notebook"
                class:chosen={picked?.includes(notebook.path)}
                class:draggingSource={drag?.active && drag.paths.includes(notebook.path)}
                role="presentation"
              >
                <button
                  type="button"
                  class="hit"
                  aria-label={$t("library.lastModified2", { "name": notebook.name, "value2": pages(notebook.pageCount), "value3": whenModified(notebook.modifiedMs) })}
                  aria-pressed={selecting ? picked?.includes(notebook.path) : undefined}
                  disabled={busy || moving !== null}
                  onpointerdown={(event) => beginEntryDrag(event, notebook)}
                  onpointermove={continueEntryDrag}
                  onpointerup={finishEntryDrag}
                  onpointercancel={cancelEntryDrag}
                  onlostpointercapture={cancelEntryDrag}
                  onclick={() => activate(notebook)}
                >
                  <NotebookCover
                    revision={coverRevision}
                    paper={notebook.paper}
                    path={notebook.path}
                    widthPx={COVER_WIDTH_PX}
                  />
                  <span class="tile-text">
                    <span class="tile-name">{#if starred.has(notebook.path)}<svg class="star" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4l2.4 5 5.6.7-4 3.9 1 5.4-5-2.7-5 2.7 1-5.4-4-3.9 5.6-.7L12 4z" /></svg>{/if}<span class="name-text">{notebook.name}</span></span>
                    <span class="tile-meta">
                      {pages(notebook.pageCount)} · {whenModified(notebook.modifiedMs)}
                    </span>
                  </span>
                </button>
                {@render tileMarks(notebook)}
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </section>
</div>

{#if drag?.active}
  <div class="drag-preview" style={dragPreviewStyle(drag)} aria-hidden="true">
    {#if drag.lead.kind === "folder"}
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
        <path
          d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2.5h7A1.5 1.5 0 0 1 19 10v7.5A1.5 1.5 0 0 1 17.5 19h-13A1.5 1.5 0 0 1 3 17.5v-10z"
          stroke="currentColor"
          stroke-width="1.5"
        />
      </svg>
    {:else}
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
        <path d="M6 3.5h8l4 4v13H6zM14 3.5v4h4" stroke="currentColor" stroke-width="1.5" />
      </svg>
    {/if}
    <span>
      {drag.paths.length === 1 ? drag.lead.name : $t("library.items2", { "length": drag.paths.length })}
    </span>
    {#if drag.paths.length > 1}<span class="drag-count">{drag.paths.length}</span>{/if}
  </div>
{/if}

{#snippet tileMarks(entry: LibraryEntry)}
  {#if selecting}
    <span class="check" class:on={picked?.includes(entry.path)} aria-hidden="true"></span>
  {:else if !moving}
    <div class="anchor tile-menu">
      <button
        type="button"
        class="chevron"
        aria-label={$t("library.actionsFor", { "name": entry.name })}
        title={$t("library.moreActions")}
        aria-haspopup="menu"
        aria-expanded={entryMenu === entry.path}
        disabled={busy}
        onclick={() => (entryMenu = entryMenu === entry.path ? null : entry.path)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="5" cy="12" r="1.35" />
          <circle cx="12" cy="12" r="1.35" />
          <circle cx="19" cy="12" r="1.35" />
        </svg>
      </button>
      {#if entryMenu === entry.path}
        <ShelfMenu
          label={$t("library.actionsFor", { "name": entry.name })}
          title={entry.name}
          items={entryMenuItems(entry)}
          onClose={() => (entryMenu = null)}
        />
      {/if}
    </div>
  {/if}
{/snippet}

{#if importing && libraryRoot}
  <ImportPanel
    bind:tool={importTool}
    sources={importSources}
    folder={path}
    folderName={crumbs.at(-1)?.name ?? $t("library.library")}
    onClose={closeImport}
    onShowFolder={(batch) => void show(batch)}
    onImported={() => void reload()}
  />
{/if}

{#if prompt?.kind === "notebook"}
  <NotebookSetup
    {busy}
    onConfirm={(setup) => void createNotebook(setup)}
    onCancel={() => (prompt = null)}
  />
{:else if prompt}
  <NamePrompt
    heading={prompt.kind === "folder"
      ? $t("library.newFolder")
      : $t("library.rename")}
    confirmLabel={prompt.kind === "rename" ? $t("library.rename") : $t("library.create")}
    initial={prompt.kind === "rename" ? prompt.initial : ""}
    {busy}
    onConfirm={(name) => void confirmPrompt(name)}
    onCancel={() => (prompt = null)}
  />
{/if}

<style>
  /* The solid language (DESIGN.md section 11): the rail is a panel on the desk, the shelf is the
     desk itself, and notebooks are the only bright things on it. */
  .library {
    position: relative;
    display: flex;
    height: 100%;
    overflow: hidden;
    background: var(--surround);
    color: var(--text);
    font-family: var(--font-ui);
    font-size: var(--text-md);
    isolation: isolate;
  }

  button {
    font: inherit;
  }

  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Rail */

  .rail {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: 2px;
    width: 248px;
    padding: 12px;
    border-right: 1px solid var(--edge-soft);
    background: var(--panel);
  }

  .brand {
    display: flex;
    gap: 10px;
    align-items: center;
    min-height: var(--control);
    margin-bottom: 12px;
    padding: 0 8px;
    font-family: var(--font-brand);
    font-size: var(--text-lg);
    font-weight: 600;
  }

  .return-to-notebook,
  .view,
  .settings-link {
    display: flex;
    gap: 12px;
    align-items: center;
    width: 100%;
    min-height: var(--control);
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    text-align: left;
    cursor: pointer;
    touch-action: manipulation;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .return-to-notebook {
    margin-bottom: 8px;
    background: var(--wash);
    color: var(--text);
  }

  .return-to-notebook:hover { background: var(--wash-strong); }

  .view:hover:not(.current),
  .settings-link:hover {
    background: var(--wash);
    color: var(--text);
  }

  .settings-link:active { background: var(--wash-strong); }

  .view.current {
    background: var(--accent-fill);
    color: var(--text);
    font-weight: 600;
  }

  .return-to-notebook svg,
  .view svg,
  .settings-link svg,
  .where svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    stroke-width: var(--stroke-dense);
  }

  .views {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .grow,
  .settings-link span {
    flex: 1;
  }

  .rail-footer {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: auto;
    padding-top: 8px;
    border-top: 1px solid var(--edge-soft);
  }

  .where {
    display: flex;
    gap: 12px;
    align-items: center;
    min-height: var(--control);
    padding: 0 4px 0 10px;
    color: var(--muted);
  }

  .where-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .link {
    flex: none;
    min-height: var(--control-dense);
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--blueprint-light);
    font-size: var(--text-sm);
    cursor: pointer;
    touch-action: manipulation;
    transition: background var(--fast) ease;
  }

  .link:hover:not(:disabled) { background: var(--wash); }

  /* Shelf */

  .shelf {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  /* Where you are is the title; the folders before it are quieter ways back up. */
  .bar {
    display: flex;
    flex: none;
    gap: 8px;
    align-items: center;
    min-height: 72px;
    padding: 0 32px;
  }

  /* The crumbs carry their own 8px padding; the outdent puts the text on the content edge. */
  .crumbs,
  .here-label {
    margin-left: -8px;
  }

  .crumbs {
    display: flex;
    gap: 2px;
    align-items: center;
    min-width: 0;
    overflow: hidden;
  }

  .crumb,
  .here-label {
    flex: none;
    min-height: var(--control);
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    transition: color var(--fast) ease, background var(--fast) ease, box-shadow var(--fast) ease;
  }

  .crumb:hover:not(:disabled) {
    background: var(--wash);
    color: var(--text);
  }

  .crumb.here,
  .here-label {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .crumb.here { cursor: default; }
  .crumb.here:hover:not(:disabled) { background: transparent; }
  .here-label { line-height: var(--control); cursor: default; }

  .crumb.drop,
  .crumb.here.drop {
    box-shadow: inset 0 0 0 1px var(--blueprint);
    background: var(--accent-fill);
    color: var(--text);
  }

  .crumb.moveTarget:not(.drop) {
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    background: var(--accent-tint);
    color: var(--text);
  }

  .separator {
    color: var(--quiet);
  }

  .spacer {
    flex: 1;
  }

  .anchor {
    position: relative;
  }

  .control,
  .primary {
    display: inline-flex;
    min-height: var(--control);
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 14px;
    border: 0;
    border-radius: var(--radius);
    white-space: nowrap;
    cursor: pointer;
    touch-action: manipulation;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .control {
    background: var(--wash);
    color: var(--text);
  }

  .control:hover:not(:disabled) { background: var(--wash-strong); }

  /* Sort and Select sit next to the primary; a fill on each would make three buttons compete. */
  .bar .control:not(.import-job) {
    background: transparent;
    color: var(--muted);
  }

  .bar .control:not(.import-job):hover:not(:disabled) {
    background: var(--wash);
    color: var(--text);
  }

  .control[aria-pressed="true"],
  .bar .control[aria-pressed="true"] {
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    background: var(--accent-fill);
    color: var(--text);
  }

  .control.destructive { color: var(--oxide-text); }
  .control.destructive:hover:not(:disabled) { background: var(--oxide-wash); }

  .primary {
    padding: 0 16px;
    background: var(--blueprint);
    color: var(--on-accent);
    font-weight: 600;
  }

  .primary:hover:not(:disabled) { background: var(--blueprint-hover); }

  .menu-control > svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    stroke-width: var(--stroke-dense);
  }

  .menu-control > svg:not(:first-child),
  .menu-control > .menu-caret {
    transition: transform var(--fast) ease-out;
  }

  .menu-control > svg.open { transform: rotate(180deg); }

  /* A running or finished import, while its panel is closed. */
  .import-job svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    stroke: var(--sage-text);
    stroke-width: 2.2;
  }

  .import-pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--blueprint);
    animation: import-pulse 1.2s ease-in-out infinite;
  }

  @keyframes import-pulse {
    50% { opacity: 0.35; }
  }

  /* Banners across the shelf: a mode (moving, selecting) or a failure. */
  .action-bar,
  .operation-error {
    display: flex;
    flex: none;
    gap: 12px;
    align-items: center;
    min-height: 48px;
    margin: 0 32px 8px;
    padding: 6px 8px 6px 16px;
    border-radius: var(--radius);
  }

  .action-bar {
    background: var(--accent-tint);
    color: var(--text);
  }

  .move-title { font-weight: 600; }
  .move-help { color: var(--muted); }

  .operation-error {
    background: var(--oxide-wash);
    color: var(--oxide-text);
  }

  .error-dismiss {
    display: grid;
    flex: none;
    width: var(--control-dense);
    height: var(--control-dense);
    padding: 0;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: currentColor;
    cursor: pointer;
    place-items: center;
  }

  .error-dismiss:hover { background: var(--wash); }

  .error-dismiss svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    stroke-width: var(--stroke-dense);
  }

  .contents {
    flex: 1;
    padding: 0 32px 40px;
    overflow-y: auto;
  }

  .contents::selection,
  .contents :global(::selection) {
    background: var(--accent-select);
    color: var(--text);
  }

  .band {
    margin: 32px 0 12px;
    color: var(--text);
    font-size: var(--text-md);
    font-weight: 600;
    line-height: 1.25;
  }

  .band:first-child { margin-top: 8px; }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, 168px);
    gap: 28px 24px;
  }

  .grid.folders {
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 8px;
  }

  .tile {
    position: relative;
    min-width: 0;
    transition: opacity var(--fast) ease;
  }

  .hit {
    display: flex;
    width: 100%;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
    touch-action: manipulation;
    user-select: none;
    -webkit-user-drag: none;
  }

  .tile-text {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .tile-name {
    display: flex;
    gap: 6px;
    align-items: center;
    min-width: 0;
    color: var(--text);
    line-height: 1.3;
  }

  .name-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tile-meta {
    overflow: hidden;
    color: var(--quiet);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .star {
    width: 14px;
    height: 14px;
    color: var(--amber);
    fill: currentColor;
    stroke: none;
  }

  /* Folders are short wide panels; notebooks are tall paper. The silhouettes differ before any
     label is read, which is what lets the two sections be scanned rather than parsed. */
  .folder {
    border-radius: var(--radius);
    background: var(--panel);
    transition: background var(--fast) ease, box-shadow var(--fast) ease, opacity var(--fast) ease;
  }

  .folder .hit {
    gap: 12px;
    align-items: center;
    min-height: 60px;
    padding: 0 48px 0 14px;
    border-radius: inherit;
    color: var(--muted);
  }

  .folder .hit > svg {
    width: var(--icon);
    height: var(--icon);
  }

  .folder:hover:not(.draggingSource):not(.drop) { background: var(--wash); }

  .folder.drop {
    box-shadow: inset 0 0 0 1px var(--blueprint);
    background: var(--accent-fill);
  }

  .folder.moveTarget:not(.drop) {
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    background: var(--accent-tint);
  }

  .notebook .hit {
    flex-direction: column;
    gap: 10px;
  }

  /* The name row leaves room for the actions button, which sits beside it, not on the paper. */
  .notebook .tile-text { padding-right: 36px; }

  .notebook .hit :global(.cover) {
    transition: box-shadow var(--fast) ease;
  }

  .notebook:hover:not(.draggingSource) .hit :global(.cover) {
    box-shadow: 0 0 0 2px var(--edge-strong);
  }

  .tile.draggingSource { opacity: var(--disabled); }

  .library.dragging,
  .library.dragging .hit {
    cursor: grabbing;
  }

  .tile.chosen::after {
    position: absolute;
    border-radius: var(--radius-lg);
    content: "";
    inset: -6px;
    box-shadow: 0 0 0 2px var(--blueprint-light);
    pointer-events: none;
  }

  .check {
    position: absolute;
    top: 8px;
    left: 8px;
    width: 20px;
    height: 20px;
    border: 1.5px solid var(--edge-strong);
    border-radius: 50%;
    background: var(--panel);
    pointer-events: none;
    transition: border-color var(--fast) ease, background var(--fast) ease;
  }

  .folder .check {
    top: 50%;
    right: 14px;
    left: auto;
    margin-top: -10px;
  }

  .check.on {
    border-color: var(--blueprint);
    background: var(--blueprint);
  }

  .check.on::after {
    position: absolute;
    top: 3px;
    left: 6px;
    width: 4px;
    height: 9px;
    border-right: 2px solid var(--on-accent);
    border-bottom: 2px solid var(--on-accent);
    content: "";
    transform: rotate(45deg);
  }

  .tile-menu {
    position: absolute;
    top: 50%;
    right: 8px;
    margin-top: calc(var(--control-dense) / -2);
  }

  .notebook .tile-menu {
    top: auto;
    right: -8px;
    bottom: 0;
    margin-top: 0;
  }

  .chevron {
    display: grid;
    width: var(--control-dense);
    height: var(--control-dense);
    padding: 0;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    place-items: center;
    touch-action: manipulation;
    transition: background var(--fast) ease, color var(--fast) ease;
  }

  .chevron:hover,
  .chevron[aria-expanded="true"] {
    background: var(--wash-strong);
    color: var(--text);
  }

  .chevron svg {
    width: var(--icon-dense);
    height: var(--icon-dense);
    fill: currentColor;
    stroke: none;
  }

  .hit:focus-visible,
  .crumb:focus-visible,
  .control:focus-visible,
  .primary:focus-visible,
  .view:focus-visible,
  .link:focus-visible,
  .chevron:focus-visible,
  .settings-link:focus-visible,
  .return-to-notebook:focus-visible,
  .error-dismiss:focus-visible {
    outline: 2px solid var(--blueprint-light);
    outline-offset: 2px;
  }

  .notebook .hit:focus-visible {
    outline: none;
  }

  .notebook .hit:focus-visible :global(.cover) {
    box-shadow: 0 0 0 2px var(--surround), 0 0 0 4px var(--blueprint-light);
  }

  /* Empty, first-run and failure states */

  .first-run {
    max-width: 460px;
    margin: 14vh auto 0;
    text-align: center;
  }

  .first-run-mark {
    display: block;
    width: 32px;
    height: 32px;
    margin: 0 auto 16px;
    color: var(--muted);
    stroke-width: var(--stroke);
  }

  .first-run h1 {
    margin: 0 0 8px;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 600;
    line-height: 1.25;
    text-wrap: balance;
  }

  .first-run p {
    margin: 0 0 24px;
    color: var(--muted);
    line-height: 1.5;
  }

  .empty {
    display: flex;
    flex-direction: column;
    gap: 16px;
    align-items: center;
    margin-top: 14vh;
    color: var(--muted);
  }

  .empty p,
  .notice {
    margin: 0;
  }

  .notice {
    margin-top: 14vh;
    color: var(--muted);
    text-align: center;
  }

  .notice.failure { color: var(--oxide-text); }

  /* Follows the pointer outside the shelf, so it is a floating surface. */
  .drag-preview {
    position: fixed;
    z-index: 1000;
    top: 0;
    left: 0;
    display: flex;
    gap: 10px;
    align-items: center;
    width: max-content;
    max-width: 240px;
    height: 40px;
    padding: 0 12px;
    border: 1px solid var(--edge);
    border-radius: var(--radius-lg);
    background: var(--panel);
    box-shadow: var(--shadow-menu);
    color: var(--text);
    font-family: var(--font-ui);
    font-size: var(--text-md);
    font-weight: 600;
    pointer-events: none;
  }

  .drag-preview > span:not(.drag-count) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .drag-count {
    display: grid;
    flex: none;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border-radius: var(--radius-pill);
    background: var(--blueprint);
    color: var(--on-accent);
    font-size: var(--text-sm);
    place-items: center;
  }

  button:disabled {
    cursor: default;
    opacity: var(--disabled);
  }

  @media (max-width: 760px) {
    .library { flex-direction: column; }

    .rail {
      width: 100%;
      flex-direction: row;
      flex-wrap: wrap;
      align-items: center;
      gap: 4px;
      padding: 8px 12px;
      border-right: 0;
      border-bottom: 1px solid var(--edge-soft);
    }

    .brand { margin: 0; padding: 0 8px 0 0; }
    .return-to-notebook { width: auto; margin: 0; }
    .views { flex-direction: row; }
    .view { width: auto; }

    .rail-footer {
      flex-direction: row;
      margin: 0 0 0 auto;
      padding: 0 0 0 8px;
      border-top: 0;
      border-left: 1px solid var(--edge-soft);
    }

    .where { padding: 0; }
    .where svg,
    .where-name { display: none; }
    .settings-link { width: auto; }
    .bar { padding: 0 16px; overflow-x: auto; }
    .contents { padding: 0 16px 28px; }
    .action-bar, .operation-error { margin: 0 16px 8px; }
  }

  @media (max-width: 520px) {
    .brand span,
    .settings-link span { display: none; }
    .settings-link { width: var(--control); padding: 0; justify-content: center; }
    .crumbs { overflow-x: auto; }
    .bar { gap: 4px; }
  }

  @media (pointer: coarse) {
    .return-to-notebook,
    .view,
    .settings-link,
    .where,
    .control,
    .primary,
    .crumb {
      min-height: var(--control-touch);
    }

    .link,
    .error-dismiss,
    .chevron {
      min-width: var(--control-touch);
      min-height: var(--control-touch);
    }

    .bar { min-height: 76px; }
  }

  @media (prefers-reduced-motion: reduce) {
    .return-to-notebook,
    .view,
    .settings-link,
    .link,
    .tile,
    .folder,
    .crumb,
    .control,
    .primary,
    .chevron,
    .menu-control > svg,
    .check,
    .notebook .hit :global(.cover) {
      transition: none;
    }

    .import-pulse { animation: none; }
  }
</style>
