<script lang="ts">
  // The standalone compose window (mounted for #/compose/{id}). It is just the
  // window chrome around the shared ComposeForm; every action that ends the
  // draft (send/discard/close) closes the window.
  import ComposeForm from "./ComposeForm.svelte";

  let { draftId }: { draftId: number } = $props();

  async function closeWindow() {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().close();
  }
</script>

<ComposeForm {draftId} chrome onSent={closeWindow} onClose={closeWindow} onDiscarded={closeWindow} />
