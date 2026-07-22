import { aiApi, type AiModel } from "./api";

/**
 * Detection proved the URL is an Ollama server, whose OpenAI API lives under
 * /v1: complete a bare root so generation doesn't 404 later (upstream
 * deliberately keeps the user's input literal otherwise).
 */
export function ollamaV1(url: string): string {
  const base = url.trim().replace(/\/+$/, "");
  return base.endsWith("/v1") ? base : `${base}/v1`;
}

/**
 * Silent Ollama detection for the custom (OpenAI-compatible) provider: if the
 * base URL turns out to belong to an Ollama server, its installed tool-capable
 * models become clickable chips. Any error (unreachable, not Ollama, …) just
 * means an empty list: no error UI, per upstream's no-connectivity-probe
 * philosophy for this provider.
 */
export function createOllamaDetection(getUrl: () => string) {
  let models = $state<AiModel[]>([]);
  let status = $state<"idle" | "some" | "none">("idle");
  // Guards against a stale response landing after a newer request started
  // (e.g. the user edits the URL again while the first lookup is in flight).
  let generation = 0;

  async function detect() {
    const url = getUrl().trim();
    if (!url) {
      // Bump the generation here too, so a still-in-flight probe for the
      // previous URL can't land afterwards and repaint chips for a field
      // that's now empty.
      ++generation;
      models = [];
      status = "idle";
      return;
    }
    const gen = ++generation;
    try {
      const found = await aiApi.ollamaModels(url);
      if (gen !== generation) return;
      models = found;
      status = found.length > 0 ? "some" : "none";
    } catch {
      if (gen !== generation) return;
      models = [];
      status = "idle";
    }
  }

  return {
    get models() {
      return models;
    },
    get status() {
      return status;
    },
    detect,
  };
}
