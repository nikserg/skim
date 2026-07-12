// New-mail toasts via the OS notification center. Fired only when the app
// window is not focused.
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { t } from "./i18n/index.svelte";

export async function initNotifications(): Promise<void> {
  await listen<{ count: number }>("mail:new", async (event) => {
    if (document.hasFocus()) return;
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (!granted) return;
    sendNotification({
      title: "Skim",
      body: t("notify.new", { n: event.payload.count }),
    });
  });
}
