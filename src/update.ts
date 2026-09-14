// The in-app updater: the check against the release feed, the download +
// install the user asked for, the footer mark and the App Update dialog's
// buttons (VersionDialog.vue reads the state). Nothing is downloaded or
// installed without a click. App.vue loads this module on demand, only
// when the backend says the install is one the updater can replace
// (`SystemInfo.updater`: the Windows installer and the AppImage; the bare
// executable and deb / rpm never call any of this).
import { markRaw, reactive, type Component } from "vue";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import UpdateMark from "./components/UpdateMark.vue";
import type { ConfirmButton } from "./components/ConfirmDialog.vue";

// idle: no check yet; current: up to date; available: an update waits for
// the user's click; downloading / installing: after it; error: the last
// check or install failed (`error` says which).
export type UpdateState = "idle" | "checking" | "current" | "available" | "downloading" | "installing" | "error";

export interface UpdateInfo {
  version: string;
  // Release date as `YYYY-MM-DD`, empty when the feed has none.
  date: string;
  // Release notes, as the feed carries them.
  notes: string;
}

export interface Updater {
  state: UpdateState;
  info: UpdateInfo | null;
  // Dev builds only (`tauri dev` has no updater): a simulated updater; the
  // dialog then carries a toggle that fakes an available update, so the GUI
  // parts can be looked at. null for a real updater.
  simulated: boolean | null;
  // Download progress 0..1; null until the size is known.
  progress: number | null;
  error: string;
  // The footer's mark while an update waits (UpdateMark.vue).
  mark: Component;
  // Check the feed; true when an update is available.
  check(): Promise<boolean>;
  // Download and install the available update, then relaunch.
  install(): Promise<void>;
  // The version dialog's update buttons for the current state.
  buttons(): ConfirmButton[];
  // Simulated updater: fake an available update, or reset.
  simulate(on: boolean): void;
}

// `simulate`: a dev-only stand-in (never talks to the plugin) instead of
// the real thing; a release build drops the simulation code.
export function createUpdater(simulate = false): Updater {
  // The plugin's handle on the update found by the last check.
  let update: Update | null = null;
  let fakeTimer: ReturnType<typeof setInterval> | null = null;

  const u: Updater = reactive({
    state: "idle" as UpdateState,
    info: null,
    simulated: import.meta.env.DEV && simulate ? false : null,
    progress: null,
    error: "",
    mark: markRaw(UpdateMark),

    async check() {
      if (import.meta.env.DEV && u.simulated !== null) {
        // A fake check: "Checking…" for a moment, then what the toggle says.
        if (u.state !== "idle" && u.state !== "current" && u.state !== "available") return false;
        u.state = "checking";
        await new Promise((r) => setTimeout(r, 900));
        u.state = u.simulated ? "available" : "current";
        return u.state === "available";
      }
      if (u.state === "checking" || u.state === "downloading" || u.state === "installing") return u.state !== "checking";
      u.state = "checking";
      u.error = "";
      try {
        const found = await check();
        await update?.close();
        update = found;
        if (!found) {
          u.info = null;
          u.state = "current";
          console.info("update check: up to date");
          return false;
        }
        u.info = { version: found.version, date: (found.date ?? "").slice(0, 10), notes: found.body ?? "" };
        u.state = "available";
        console.info(`update check: v${found.version} available (running v${found.currentVersion})`);
        return true;
      } catch (e) {
        u.error = `Check failed: ${e}`;
        u.state = "error";
        console.warn("update check failed", e);
        return false;
      }
    },

    async install() {
      if (u.state !== "available") return;
      if (import.meta.env.DEV && u.simulated !== null) {
        // A fake download: 2 % every 60 ms, then "installing" for good.
        u.state = "downloading";
        u.progress = 0;
        fakeTimer = setInterval(() => {
          u.progress = Math.min(1, (u.progress ?? 0) + 0.02);
          if (u.progress >= 1) {
            if (fakeTimer) clearInterval(fakeTimer);
            fakeTimer = null;
            u.state = "installing";
          }
        }, 60);
        return;
      }
      if (!update) return;
      u.state = "downloading";
      u.progress = null;
      u.error = "";
      let total = 0;
      let done = 0;
      try {
        await update.downloadAndInstall((ev) => {
          if (ev.event === "Started") {
            total = ev.data.contentLength ?? 0;
            u.progress = total ? 0 : null;
          } else if (ev.event === "Progress") {
            done += ev.data.chunkLength;
            if (total) u.progress = Math.min(1, done / total);
          } else if (ev.event === "Finished") {
            u.state = "installing";
          }
        });
        console.info(`update v${update.version} installed, relaunching`);
        await relaunch();
      } catch (e) {
        u.error = `Install failed: ${e}`;
        u.state = "error";
        console.error("update install failed", e);
      }
    },

    buttons() {
      const busy = u.state === "checking" || u.state === "downloading" || u.state === "installing";
      // Check Update until one is found; Install from then on.
      if (u.state === "available" || u.state === "downloading" || u.state === "installing") {
        return [{ label: "Install", kind: "primary", value: "install", disabled: busy }];
      }
      return [{ label: "Check Update", kind: "outline", value: "check", disabled: busy }];
    },

    simulate(on) {
      if (!import.meta.env.DEV || u.simulated === null) return;
      if (fakeTimer) clearInterval(fakeTimer);
      fakeTimer = null;
      u.simulated = on;
      u.progress = null;
      u.error = "";
      if (on) {
        u.info = {
          version: "9.9.9",
          date: new Date().toISOString().slice(0, 10),
          notes: "Simulated release notes.\n\n- A first change\n- A second change",
        };
        u.state = "available";
      } else {
        u.info = null;
        u.state = "idle";
      }
    },
  });
  return u;
}
