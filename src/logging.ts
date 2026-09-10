// Forward the webview console and uncaught errors to the backend logger, so
// they land in bindsight.log next to the Rust side (target "webview").
//
// Calls the tauri-plugin-log command directly instead of using its JS
// package: the package tags every record with the caller's source location
// (`webview:App.vue:123`), which the backend's per-target level filter cannot
// match; a plain "webview" target can.
import { invoke } from "@tauri-apps/api/core";

// tauri-plugin-log's LogLevel.
const LEVEL = { trace: 1, debug: 2, info: 3, warn: 4, error: 5 } as const;
type Level = keyof typeof LEVEL;

function fmt(args: unknown[]): string {
  return args
    .map((a) => {
      if (a instanceof Error) return a.stack ?? `${a.name}: ${a.message}`;
      if (typeof a === "string") return a;
      try {
        return JSON.stringify(a);
      } catch {
        return String(a);
      }
    })
    .join(" ");
}

// Settings "Enable debug logging". The backend caps its own level, but the
// plugin's log command bypasses that cap, so debug records are dropped here.
let debugEnabled = false;

export function setDebugLogging(enabled: boolean) {
  debugEnabled = enabled;
}

function send(level: Level, message: string) {
  if (LEVEL[level] < LEVEL.info && !debugEnabled) return;
  // Never let a logging failure surface (it would recurse into console.error).
  invoke("plugin:log|log", { level: LEVEL[level], message }).catch(() => {});
}

export function forwardConsoleToLog() {
  const hook = (name: "debug" | "log" | "info" | "warn" | "error", level: Level) => {
    const original = console[name].bind(console);
    console[name] = (...args: unknown[]) => {
      original(...args);
      send(level, fmt(args));
    };
  };
  hook("debug", "debug");
  hook("log", "info");
  hook("info", "info");
  hook("warn", "warn");
  hook("error", "error");

  window.addEventListener("error", (e) => {
    send("error", `uncaught: ${fmt([e.error ?? e.message])} (${e.filename}:${e.lineno})`);
  });
  window.addEventListener("unhandledrejection", (e) => {
    send("error", `unhandled rejection: ${fmt([e.reason])}`);
  });
}
