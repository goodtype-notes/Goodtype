<script lang="ts">
  import type { Block, Span } from "../../ipc/addons";
  let { blocks }: { blocks: Block[] } = $props();
</script>

{#snippet styled(span: Span)}
  {#if span.code}<code>{span.text}</code>{:else}{span.text}{/if}
{/snippet}
{#snippet emphasis(span: Span)}
  {#if span.em}<em>{@render styled(span)}</em>{:else}{@render styled(span)}{/if}
{/snippet}
{#snippet inline(spans: Span[])}
  {#each spans as span}
    {#if span.strong}<strong>{@render emphasis(span)}</strong>{:else}{@render emphasis(span)}{/if}
  {/each}
{/snippet}
{#snippet listItems(items: Block[][])}
  {#each items as item}<li>{@render renderBlocks(item)}</li>{/each}
{/snippet}
{#snippet renderBlocks(items: Block[])}
  {#each items as block}
    {#if block.kind === "heading"}
      {#if block.level === 2}<h2>{@render inline(block.spans)}</h2>{:else}<h3>{@render inline(block.spans)}</h3>{/if}
    {:else if block.kind === "paragraph"}
      <p>{@render inline(block.spans)}</p>
    {:else if block.kind === "list"}
      {#if block.ordered}<ol start={block.start ?? undefined}>{@render listItems(block.items)}</ol>
      {:else}<ul>{@render listItems(block.items)}</ul>{/if}
    {:else if block.kind === "code"}
      <pre><code>{block.text}</code></pre>
    {:else if block.kind === "quote"}
      <blockquote>{@render renderBlocks(block.blocks)}</blockquote>
    {:else if block.kind === "rule"}
      <hr />
    {:else if block.kind === "table"}
      <div class="table-scroll">
        <table>
          <thead><tr>{#each block.head as cell}<th scope="col">{@render inline(cell)}</th>{/each}</tr></thead>
          <tbody>{#each block.rows as row}<tr>{#each row as cell}<td>{@render inline(cell)}</td>{/each}</tr>{/each}</tbody>
        </table>
      </div>
    {/if}
  {/each}
{/snippet}

<div class="readme" data-ui="solid">{@render renderBlocks(blocks)}</div>

<style>
  .readme { max-width: 70ch; color: var(--text); font-size: var(--text-md); line-height: 1.6; overflow-wrap: anywhere; }
  h2, h3 { margin: 24px 0 12px; font-weight: 600; line-height: 1.25; }
  .readme > :global(:first-child) { margin-top: 0; }
  h2 { font-size: var(--text-lg); }
  h3 { font-size: var(--text-md); }
  p, ul, ol, pre, blockquote, .table-scroll { margin: 0 0 12px; }
  ul, ol { padding-inline-start: 24px; }
  li > p { margin: 0; }
  li > ul, li > ol { margin-bottom: 0; }
  strong { font-weight: 600; }
  code { padding: 1px 4px; background: var(--panel); border-radius: var(--radius); font-family: var(--font-code); font-size: var(--text-sm); }
  pre { padding: 12px; background: var(--inset); border-radius: var(--radius); overflow-x: auto; }
  pre code { padding: 0; background: transparent; font-size: var(--text-sm); }
  blockquote { margin-inline-start: 16px; color: var(--muted); }
  hr { border: 0; border-top: 1px solid var(--edge-soft); margin: 16px 0; }
  .table-scroll { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; text-align: start; }
  th, td { padding: 8px 12px; border-bottom: 1px solid var(--edge-soft); text-align: start; }
  th { color: var(--muted); font-weight: 600; }
</style>
