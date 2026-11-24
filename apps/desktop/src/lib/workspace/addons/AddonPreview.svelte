<script lang="ts">
  let { kind, image, percent = null, busy = false, large = false }: {
    kind: string | undefined; image?: string | null; percent?: number | null; busy?: boolean; large?: boolean;
  } = $props();
  let failedImage = $state<string>();
  const safeImage = $derived(image && image !== failedImage && /^data:image\/(png|jpeg|webp);base64,/i.test(image) ? image : null);
</script>

<div class="preview" class:large aria-hidden="true">
  {#if safeImage}
    <img src={safeImage} alt="" onerror={() => { failedImage = safeImage ?? undefined; }} />
  {:else}
    <svg viewBox="0 0 72 48" fill="none" stroke-linecap="round" stroke-linejoin="round">
      {#if kind === "import"}
        <path class="secondary" d="M30 8h23l7 7v24m-35-3v6h29M49 12V5h9l7 7v23" />
        <path d="M25 21V12h21l8 8v16H31m15-24v8h8M7 28h29m-6-5 6 5-6 5m9-8h9m-9 5h9" />
      {:else if kind === "recognize"}
        <path d="M5 31c4-3 10-14 12-16 2-1 0 6-1 11-2 7 3 5 6 0m17 2 4 4 6-15h18m-14 7 9 8m0-8-9 8" />
        <path class="secondary" d="M27 24h9m-3-3 3 3-3 3M45 37h21" />
      {:else}
        <path class="secondary" d="M17 14 36 5l19 9v20l-19 9-19-9z" />
        <path d="m17 14 19 9 19-9m-19 9v20M26 10l19 9v10" />
      {/if}
    </svg>
  {/if}
  {#if busy}<span class="progress" class:indeterminate={percent === null} style:width={percent === null ? "30%" : percent + "%"}></span>{/if}
</div>

<style>
  .preview { position: relative; display: grid; place-items: center; aspect-ratio: 16 / 10; background: var(--charcoal); border-radius: var(--radius); overflow: hidden; }
  .large { aspect-ratio: 16 / 9; border: 1px solid var(--edge); border-radius: var(--radius-lg); }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  svg { width: 100px; max-width: 44%; color: var(--muted); stroke: currentColor; stroke-width: 1.2; }
  .large svg { width: 150px; }
  .secondary { stroke: var(--edge-strong); }
  .progress { position: absolute; bottom: 0; left: 0; height: 2px; background: var(--blueprint); }
  .indeterminate { animation: progress 1.4s ease-in-out infinite alternate; }
  @keyframes progress { to { left: 70%; } }
  @media (prefers-reduced-motion: reduce) { .indeterminate { animation: none; } }
</style>
