import { render } from "svelte/server";
import { describe, expect, it } from "vitest";
import type { ManagedAddon, Span } from "../../ipc/addons";
import ReadmeView from "./ReadmeView.svelte";
import { addonAction, matchesAddon } from "./storeModel";

const release = { version: "1.2.0", sizeBytes: 1024 };
const addon: ManagedAddon = {
  id: "fixture", name: { en: "Math recognition", de: "Formelerkennung" },
  summary: { en: "Read equations", de: "Gleichungen lesen" }, repository: "owner/fixture",
  installed: null, previous: null, status: "available", expectedApi: "1", foundApi: null,
  permissions: [], catalogPermissions: ["selected-ink"], formats: [], memoryBytes: 1024,
  timeoutSeconds: 10, outputBytes: 1024, isolation: "sandbox", release, update: null,
  directApply: false, activity: "idle", approval: null, progress: null,
};
const span = (text: string, modifiers: Partial<Span> = {}): Span =>
  ({ text, href: null, strong: false, em: false, code: false, ...modifiers });

describe("addon store", () => {
  it("matches only the name in the current language, ignoring case and surrounding whitespace", () => {
    expect(matchesAddon(addon, "  MATH ", "en")).toBe(true);
    expect(matchesAddon(addon, "FORMEL", "de")).toBe(true);
    expect(matchesAddon(addon, "math", "de")).toBe(false);
    expect(matchesAddon(addon, "equations", "en")).toBe(false);
    expect(matchesAddon(addon, "Gleichungen", "de")).toBe(false);
  });

  it.each([
    [{}, "install"],
    [{ status: "needs-reinstall" }, "reinstall"],
    [{ status: "needs-reinstall", release: null }, null],
    [{ status: "enabled", update: release }, "update"],
    [{ status: "enabled", update: release, activity: "downloading" }, "cancel"],
    [{ activity: "installing" }, "cancel"],
    [{ status: "enabled" }, null],
    [{ status: "disabled" }, null],
    [{ activity: "awaitingApproval" }, null],
  ] satisfies [Partial<ManagedAddon>, string | null][])("maps addon state %j to %s", (changes, expected) => {
    expect(addonAction({ ...addon, ...changes })).toBe(expected);
  });

  it("renders README structure, escapes literal HTML and discards link destinations", () => {
    const { body } = render(ReadmeView, { props: { blocks: [
      { kind: "heading", level: 2, spans: [span("Setup")] },
      { kind: "heading", level: 3, spans: [span("Input")] },
      { kind: "paragraph", spans: [
        span('<img src=x onerror="alert(1)">', { href: "javascript:alert(1)" }),
        span("sample", { strong: true, em: true, code: true }),
      ] },
      { kind: "list", ordered: true, start: 3, items: [[{ kind: "paragraph", spans: [span("Step")] }]] },
      { kind: "quote", blocks: [{ kind: "paragraph", spans: [span("Quote")] }] },
      { kind: "code", text: "<script>text</script>" },
      { kind: "table", head: [[span("Format")]], rows: [[[span("PDF")]]] },
      { kind: "rule" },
    ] } });
    for (const tag of ["h2", "h3", "p", "strong", "em", "code", "li", "blockquote", "pre", "table", "hr"]) {
      expect(body).toContain("<" + tag);
    }
    expect(body).toContain('<ol start="3"');
    expect(body).toContain("&lt;img");
    expect(body).toContain("&lt;script>");
    expect(body).not.toMatch(/<img|<script|href=|<a\b/);
  });
});
