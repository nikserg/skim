import { ai } from "./stores/ai.svelte";

/**
 * A custom endpoint's cold start can stall the first token for many seconds:
 * after 5s of silence, say what is actually happening. Only the custom
 * (OpenAI-compatible, often local) provider warrants the hint; cloud
 * providers answer fast or fail fast. The provider is read at arm time.
 */
export function createSlowStart() {
  let slow = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    get slow() {
      return slow;
    },
    arm() {
      clearTimeout(timer);
      slow = false;
      if (ai.provider !== "custom") return;
      timer = setTimeout(() => (slow = true), 5000);
    },
    clear() {
      clearTimeout(timer);
      slow = false;
    },
  };
}
