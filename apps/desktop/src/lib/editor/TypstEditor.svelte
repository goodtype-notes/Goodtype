<script lang="ts">
  import { extractFocus } from "../helper/editorEntry";
  import { t, notice, renderMessage, type UiText } from "../i18n";
  import { acceptCompletion, autocompletion } from "@codemirror/autocomplete";
  import { setDiagnostics, type Diagnostic } from "@codemirror/lint";
  import { keymap, tooltips } from "@codemirror/view";
  import { Compartment, EditorState, Prec } from "@codemirror/state";
  import { editorPhrases } from "./phrases";
  import { basicSetup, EditorView } from "codemirror";
  import { onMount } from "svelte";
  import { byteOffsetMapper, createTypstCompletionSource, fromByteOffset, toByteOffset } from "./completion";
  import { setTypstHighlights, typstHighlighting } from "./highlighting";
  import { newlineInsidePair, spaceInsideMath, typstPairLanguageData } from "./smartPairs";
  import { typstSnippetSource } from "./snippets";
  import { applyWritingCommand as runWritingCommand, type WritingCommand } from "./writingCommands";
  import { analyzeTypst, formatTypst, hoverTypst, type TypstHover } from "../ipc/typst";

  let {
    value,
    root = null,
    ariaLabel = "",
    maxLines = 10,
    lineWrap = false,
    onChange,
    onExit,
    onLineWrapToggle,
    onAskAI,
  }: {
    value: string;
    /** Notebook root, needed to ask Rust for compiler-derived completions. */
    root?: string | null;
    ariaLabel?: string;
    /** Height ceiling, in lines. `null` fills the available height (the side view). */
    maxLines?: number | null;
    /** Soft-wrap long visual lines without changing the source document. */
    lineWrap?: boolean;
    onChange: (value: string) => void;
    onExit: () => void;
    onLineWrapToggle?: () => void;
    onAskAI?: () => void;
  } = $props();

  let host: HTMLDivElement;

  let view: EditorView | undefined;
  let applyingExternalValue = false;
  let highlightTimer: ReturnType<typeof setTimeout> | undefined;
  let highlightGeneration = 0;
  let formatGeneration = 0;
  let help = $state<TypstHover | null>(null);
  let helpStatus = $state<UiText>("");
  const lineWrapCompartment = new Compartment();
  const languageCompartment = new Compartment();
  const effectiveAriaLabel = $derived(ariaLabel || $t("editor.typstSource"));
  const lineCount = $derived(value ? value.split("\n").length : 1);
  const hiddenLines = $derived(
    maxLines === null ? 0 : Math.max(0, lineCount - maxLines),
  );

  onMount(() => {
    view = new EditorView({
      doc: value,
      extensions: [
        basicSetup,
        lineWrapCompartment.of(lineWrap ? EditorView.lineWrapping : []),
        typstPairLanguageData,
        typstHighlighting,
        // Compiler-derived completion alongside the multi-line STEM templates, which
        // the compiler has no equivalent for.
        autocompletion({
          override: [createTypstCompletionSource(() => root), typstSnippetSource],
        }),
        // The block editor lives inside the page, which clips its overflow and stacks the ink
        // layer above it. Rendering the completion popup into the body escapes both, so it can
        // sit above the editor, the rendered preview, and everything else while you type.
        tooltips({ parent: document.body, position: "fixed" }),
        // Declares the editor dark so CodeMirror applies its dark base theme. Without this its
        // `&light .cm-tooltip` rule paints the completion popup near-white, and that rule is
        // more specific than any plain `.cm-tooltip` override we could write.
        EditorView.theme({}, { dark: true }),
        // Tab accepts the highlighted candidate; when no completion is open this returns false
        // so Tab falls through to snippet-field navigation and then to indentation.
        Prec.highest(
          keymap.of([
            { key: "Mod-Shift-Space", run: () => { if (!onAskAI) return false; onAskAI(); return true; } },
            { key: "Tab", run: acceptCompletion },
            { key: "Space", run: spaceInsideMath },
            { key: "Enter", run: newlineInsidePair },
            { key: "Mod-b", run: (target) => runWritingCommand(target, "bold") },
            { key: "Mod-i", run: (target) => runWritingCommand(target, "italic") },
            { key: "Mod-u", run: (target) => runWritingCommand(target, "underline") },
            { key: "Mod-Alt-1", run: (target) => runWritingCommand(target, "heading-1") },
            { key: "Mod-Alt-2", run: (target) => runWritingCommand(target, "heading-2") },
            { key: "Mod-Alt-3", run: (target) => runWritingCommand(target, "heading-3") },
            {
              key: "Alt-z",
              run: () => {
                if (!onLineWrapToggle) return false;
                onLineWrapToggle();
                return true;
              },
            },
            {
              key: "F1",
              run: () => {
                void showHelp();
                return true;
              },
            },
            {
              key: "Mod-Shift-f",
              run: () => {
                void formatDocument();
                return true;
              },
            },
          ]),
        ),
        languageCompartment.of([
          EditorView.contentAttributes.of({ "aria-label": effectiveAriaLabel }),
          EditorState.phrases.of(editorPhrases($t)),
        ]),
        EditorView.domEventHandlers({
          keydown(event) {
            if (event.key !== "Escape") return false;
            onExit();
            return true;
          },
        }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            const source = update.state.doc.toString();
            scheduleHighlight(source);
            if (applyingExternalValue) return;
            help = null;
            helpStatus = "";
            onChange(source);
          }
        }),
      ],
      parent: host,
    });
    view.focus();
    scheduleHighlight(value);

    return () => {
      clearTimeout(highlightTimer);
      highlightGeneration += 1;
      view?.destroy();
      view = undefined;
    };
  });

  /// Lets the side view take the caret back after a pen stroke, so drawing on the canvas never
  /// costs the writer their place in the source.
  export function getFocus() { return view ? extractFocus(view.state) : null; }

  export function focus() {
    view?.focus();
  }

  export function applyWritingCommand(command: WritingCommand) {
    if (view) runWritingCommand(view, command);
  }

  export async function showHelp() {
    if (!view || !root) return;
    const source = view.state.doc.toString();
    helpStatus = notice("editor.lookingUpTypstHelp");
    try {
      help = await hoverTypst(
        root,
        source,
        toByteOffset(source, view.state.selection.main.head),
      );
      helpStatus = help ? "" : notice("editor.noTypstHelpAtTheCaret");
    } catch {
      help = null;
      helpStatus = notice("editor.typstHelpIsUnavailable");
    }
  }

  export async function formatDocument() {
    if (!view || !root) return;
    const editor = view;
    const document = editor.state.doc;
    const source = document.toString();
    const requestedRoot = root;
    const generation = ++formatGeneration;
    const current = () => generation === formatGeneration && view === editor &&
      root === requestedRoot && editor.state.doc === document;
    helpStatus = notice("editor.formattingTypst");
    try {
      const formatted = await formatTypst(requestedRoot, source);
      if (!current()) return;
      if (formatted !== source) {
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: formatted },
        });
      }
      helpStatus = formatted === source ? notice("editor.typstSourceIsAlreadyFormatted") : notice("editor.formattedTypstSource");
    } catch {
      if (!current()) return;
      helpStatus = notice("editor.typstFormattingFailed");
    }
  }

  function scheduleHighlight(source: string) {
    clearTimeout(highlightTimer);
    const generation = ++highlightGeneration;
    const requestedRoot = root;
    if (!requestedRoot || !view) {
      clearAnalysis();
      return;
    }
    highlightTimer = setTimeout(async () => {
      try {
        const analysis = await analyzeTypst(requestedRoot, source);
        if (
          generation !== highlightGeneration ||
          root !== requestedRoot ||
          view?.state.doc.toString() !== source
        ) return;
        // No analyzer answered — keep whatever is already coloured rather than blanking the
        // document. An empty result from a working analyzer is a real answer and does clear it.
        if (!analysis.available) return;
        const offset = byteOffsetMapper(source);
        const diagnostics: Diagnostic[] = analysis.diagnostics.map((diagnostic) => ({
          severity: diagnostic.severity,
          message: diagnostic.message,
          from: offset(diagnostic.from),
          to: offset(diagnostic.to),
        }));
        view.dispatch(
          {
            effects: setTypstHighlights.of(analysis.highlights.map((token) => ({
              kind: token.kind,
              modifiers: token.modifiers,
              from: offset(token.from),
              to: offset(token.to),
            }))),
          },
          setDiagnostics(view.state, diagnostics),
        );
      } catch {
        // Highlighting is an assist; stale colors map with edits until the next successful pass.
      }
    }, 90);
  }

  function clearAnalysis() {
    if (!view) return;
    view.dispatch(
      { effects: setTypstHighlights.of([]) },
      setDiagnostics(view.state, []),
    );
  }

  $effect(() => {
    if (!view || view.state.doc.toString() === value) return;
    applyingExternalValue = true;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: value },
    });
    applyingExternalValue = false;
  });

  $effect(() => {
    const hasRoot = Boolean(root);
    if (!view) return;
    if (hasRoot) scheduleHighlight(view.state.doc.toString());
    else clearAnalysis();
  });

  $effect(() => {
    if (!view) return;
    view.dispatch({
      effects: lineWrapCompartment.reconfigure(lineWrap ? EditorView.lineWrapping : []),
    });
  });

  $effect(() => {
    const phrases = editorPhrases($t);
    const label = effectiveAriaLabel;
    if (!view) return;
    view.dispatch({ effects: languageCompartment.reconfigure([
      EditorState.phrases.of(phrases),
      EditorView.contentAttributes.of({ "aria-label": label }),
    ]) });
  });
</script>

<div class="editor" class:filled={maxLines === null} style:--max-lines={maxLines ?? 0}>
  <div class="host" bind:this={host}></div>
  {#if help || helpStatus}
    <div class:code={help?.code} class="help" role="status" aria-live="polite">
      {renderMessage(help?.value ?? helpStatus, $t)}
    </div>
  {/if}
  {#if hiddenLines > 0}
    <!-- The box is capped on purpose, so say so rather than looking truncated. -->
    <p class="overflow-hint">
      {$t("counts.hiddenLines", { count: hiddenLines })}
    </p>
  {/if}
</div>

<style>
  .editor {
    min-width: 18rem;
    max-width: 42rem;
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-panel);
    text-align: left;
    /* It floats over the page now rather than sitting flush under the block, so it carries its
       own opaque surface instead of borrowing the block's. */
    overflow: hidden;
  }

  /* Side view: fill the panel instead of floating with a capped height. */
  .editor.filled {
    display: flex;
    max-width: none;
    min-width: 0;
    height: 100%;
    flex-direction: column;
    border-radius: 0;
    box-shadow: none;
  }

  .editor.filled .host {
    min-height: 0;
    flex: 1;
  }

  .editor.filled :global(.cm-editor) {
    max-height: none;
    height: 100%;
    border: 0;
  }

  .editor :global(.cm-editor) {
    /* Capped at `maxLines`; longer sources belong in the side view. 1.65 is the line-height
       set below, and the addend covers the editor's own vertical padding. */
    max-height: calc(var(--max-lines) * 1.65 * 12px + 10px);
    border: 1px solid var(--edge);
    background: var(--inset);
    color: var(--text);
    font: 12px/1.65 var(--font-code);
  }

  .editor :global(.cm-editor.cm-focused) {
    outline: 2px solid var(--blueprint-light);
    outline-offset: -2px;
  }

  .editor :global(.cm-scroller) {
    overflow: auto;
  }

  .overflow-hint {
    margin: 0;
    padding: 8px 12px;
    border: 1px solid var(--edge);
    border-top: 0;
    background: var(--surround);
    color: var(--muted);
    font: var(--text-sm)/1.4 var(--font-ui);
  }

  .help {
    padding: 8px 12px;
    border: 1px solid var(--edge);
    border-top: 0;
    background: var(--surround);
    color: var(--muted);
    font: var(--text-sm)/1.4 var(--font-ui);
  }

  .help.code {
    font-family: var(--font-code);
  }

  .editor :global(.cm-gutters) {
    border-right: 1px solid var(--edge-soft);
    background: var(--inset);
    color: var(--quiet);
  }

  .editor :global(.cm-activeLine),
  .editor :global(.cm-activeLineGutter) {
    background: var(--fill-soft);
  }

  .editor :global(.cm-activeLineGutter) {
    color: var(--muted);
  }

  .editor :global(.cm-cursor) {
    border-left-color: var(--blueprint-light);
  }

  .editor :global(.cm-selectionBackground) {
    background: var(--accent-select);
  }

  /* Syntax colours are content, not interface. */
  .editor :global(.cm-typst-comment) { color: #737d89; font-style: italic; }
  .editor :global(.cm-typst-string) { color: #a8d1a0; }
  .editor :global(.cm-typst-keyword) { color: #c6a0f6; }
  .editor :global(.cm-typst-number),
  .editor :global(.cm-typst-bool) { color: #f4b76e; }
  .editor :global(.cm-typst-function) { color: var(--blueprint-light); }
  .editor :global(.cm-typst-type),
  .editor :global(.cm-typst-namespace) { color: #e3a6e8; }
  .editor :global(.cm-typst-label),
  .editor :global(.cm-typst-ref),
  .editor :global(.cm-typst-link),
  .editor :global(.cm-typst-decorator) { color: #75d2c6; }
  .editor :global(.cm-typst-heading) { color: #f1d17a; font-weight: 600; }
  .editor :global(.cm-typst-raw) { color: #d6a872; }
  .editor :global(.cm-typst-escape) { color: #f08c82; }
  .editor :global(.cm-typst-error) {
    text-decoration: underline wavy var(--oxide);
    text-underline-offset: 2px;
  }
  .editor :global(.cm-typst-math) { color: #8fc8ff; }
  .editor :global(.cm-lintRange-error) {
    background-image: none;
    background: var(--oxide-wash);
    text-decoration: underline wavy var(--oxide);
    text-decoration-thickness: 1.5px;
    text-underline-offset: 3px;
  }
  .editor :global(.cm-lintRange-warning) {
    background-image: none;
    text-decoration: underline wavy var(--amber);
    text-underline-offset: 3px;
  }

  /* Tooltips render into the document body, so they are not descendants of `.editor` and these
     rules are global. The popup needs a fully
     opaque surface of its own. `!important` on the two properties CodeMirror's base theme sets
     itself: its `&dark .cm-tooltip` rule is more specific than anything written from outside. */
  :global(.cm-tooltip) {
    /* Above the page, the ink layer, the palette, and the rest of the workspace chrome. */
    z-index: 2000;
    border: 1px solid var(--edge) !important;
    border-radius: var(--radius-lg);
    background: var(--panel) !important;
    box-shadow: var(--shadow-menu);
    color: var(--text);
    font-family: var(--font-ui);
    font-size: var(--text-md);
  }

  :global(.cm-tooltip-autocomplete > ul) {
    max-height: 16rem;
    margin: 0;
    padding: 4px;
    /* Monospace, because every candidate is inserted as code. */
    font: 12px/1.4 var(--font-code);
  }

  :global(.cm-tooltip-autocomplete > ul > li) {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 4px 8px;
    border-radius: var(--radius);
    color: var(--text);
  }

  :global(.cm-tooltip-autocomplete > ul > li:hover:not([aria-selected])) {
    background: var(--wash);
  }

  :global(.cm-tooltip-autocomplete > ul > li[aria-selected]) {
    background: var(--accent-fill);
    box-shadow: inset 0 0 0 1px var(--accent-edge);
    color: var(--text);
  }

  :global(.cm-completionLabel) {
    flex: 1;
  }

  /* The matched characters, so it is obvious why a candidate is in the list. */
  :global(.cm-completionMatchedText) {
    color: var(--blueprint-light);
    text-decoration: none;
    font-weight: 600;
  }

  /* Carries the glyph for a math symbol (`∑`), so it must stay legible when not selected. */
  :global(.cm-completionDetail) {
    color: var(--muted);
    font-style: normal;
  }

  :global(.cm-completionIcon) {
    width: 1.1em;
    padding-right: 0;
  }

  :global(.cm-tooltip.cm-completionInfo) {
    margin-left: 4px;
    padding: 8px;
  }
</style>
