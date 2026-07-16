import "@fontsource-variable/hanken-grotesk";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "./styles/tokens.css";
import "./styles/base.css";
import { mount } from "svelte";
import App from "./App.svelte";
import ComposeRoot from "./ComposeRoot.svelte";

// Suppress the WebView2 default context menu (Back / Reload / Save as / Print…):
// it leaks the browser underneath and has no place in a native mail client.
// Editable fields keep their menu so right-click paste/copy still works.
document.addEventListener("contextmenu", (e) => {
  const el = e.target;
  const editable =
    el instanceof HTMLInputElement ||
    el instanceof HTMLTextAreaElement ||
    (el instanceof HTMLElement && el.isContentEditable);
  if (!editable) e.preventDefault();
});

const composeMatch = window.location.hash.match(/^#\/compose\/(\d+)$/);

const app = composeMatch
  ? mount(ComposeRoot, {
      target: document.getElementById("app")!,
      props: { draftId: Number(composeMatch[1]) },
    })
  : mount(App, {
      target: document.getElementById("app")!,
    });

export default app;
