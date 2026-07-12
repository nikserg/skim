import "@fontsource-variable/hanken-grotesk";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "./styles/tokens.css";
import "./styles/base.css";
import { mount } from "svelte";
import App from "./App.svelte";
import ComposeRoot from "./ComposeRoot.svelte";

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
