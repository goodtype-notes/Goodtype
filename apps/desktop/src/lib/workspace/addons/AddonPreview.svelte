<script lang="ts">
  /**
   * A preview image supplied by an addon. Without one nothing is drawn: an empty well says
   * nothing, so the caller shows the addon's text instead.
   */
  let { kind, image, percent = null, busy = false, large = false }: {
    kind: string | undefined; image?: string | null; percent?: number | null; busy?: boolean; large?: boolean;
  } = $props();
  let failedImage = $state<string>();
  const safeImage = $derived(image && image !== failedImage && /^data:image\/(png|jpeg|webp);base64,/i.test(image) ? image : null);
</script>

{#if safeImage}
  <div class="preview" class:large data-ui="solid" data-kind={kind} aria-hidden="true">
    <img src={safeImage} alt="" onerror={() => { failedImage = safeImage ?? undefined; }} />
    {#if busy}<span class="progress" class:indeterminate={percent === null} style:width={percent === null ? "30%" : percent + "%"}></span>{/if}
  </div>
{/if}

<style>
  .preview { position: relative; display: grid; place-items: center; aspect-ratio: 16 / 10; margin-bottom: 16px; background: var(--paper); border-radius: var(--radius); overflow: hidden; }
  .large { aspect-ratio: 16 / 9; border-radius: var(--radius-lg); }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .progress { position: absolute; bottom: 0; left: 0; height: 2px; background: var(--blueprint); }
  .indeterminate { animation: progress 1.4s ease-in-out infinite alternate; }
  @keyframes progress { to { opacity: 0.4; } }
  @media (prefers-reduced-motion: reduce) { .indeterminate { animation: none; } }
</style>
