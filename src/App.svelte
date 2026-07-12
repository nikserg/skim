<script lang="ts">
  import Titlebar from "./components/Titlebar.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import MessageList from "./components/MessageList.svelte";
  import ReadingPane from "./components/ReadingPane.svelte";
  import Onboarding from "./components/onboarding/Onboarding.svelte";
  import { mail } from "./lib/stores/mail.svelte";
  import { setLocale } from "./lib/i18n/index.svelte";
  import { api } from "./lib/api";
  import { ui } from "./lib/stores/ui.svelte";
  import type { Account, Theme } from "./lib/types";

  let ready = $state(false);

  $effect(() => {
    void (async () => {
      const inTauri = "__TAURI_INTERNALS__" in window;
      if (inTauri) {
        try {
          const settings = await api.getSettings();
          if (settings.locale) await setLocale(settings.locale as never);
          if (settings.theme) ui.setTheme(settings.theme as Theme);
        } catch {
          // settings are best-effort at boot
        }
        await mail.boot();
      }
      ready = true;
    })();
  });

  function onboarded(account: Account) {
    void mail.accountAdded(account);
  }
</script>

<div class="app">
  <Titlebar />
  {#if ready}
    {#if mail.account}
      <main class="panes">
        <Sidebar />
        <MessageList />
        <ReadingPane />
      </main>
    {:else}
      <Onboarding oncomplete={onboarded} />
    {/if}
  {/if}
</div>

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .panes {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
