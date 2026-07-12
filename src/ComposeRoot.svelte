<script lang="ts">
  // Root for compose windows: minimal boot (settings → theme/locale), then
  // the composer itself.
  import Composer from "./components/Composer.svelte";
  import { api } from "./lib/api";
  import { setLocale } from "./lib/i18n/index.svelte";
  import { ui } from "./lib/stores/ui.svelte";
  import type { Theme } from "./lib/types";

  let { draftId }: { draftId: number } = $props();
  let ready = $state(false);

  $effect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings();
        if (settings.locale) await setLocale(settings.locale as never);
        if (settings.theme) ui.setTheme(settings.theme as Theme);
      } catch {
        // best effort
      }
      ready = true;
    })();
  });
</script>

{#if ready}
  <Composer {draftId} />
{/if}
