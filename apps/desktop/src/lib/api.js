// The IPC boundary.
//
// Every call goes through call(), so a "locked" rejection from the backend
// — the vault auto-locked, or was locked from another code path — sends the
// UI to the unlock screen instead of surfacing as a confusing error in
// whatever view happened to be open.
//
// It is also where a response that outlived its session is dropped. A
// request made before a lock can complete after it; delivering that result
// would let decrypted data repopulate a cache or open a dialog on a locked
// screen. Each lock ends the session generation, and anything from an older
// generation is rejected as locked instead.

import { invoke } from "@tauri-apps/api/core";

let onLocked = () => {};
let generation = 0;

export function setLockedHandler(fn) {
  onLocked = fn;
}

/** Called on every lock: responses to earlier requests are discarded. */
export function endSession() {
  generation += 1;
}

function stale() {
  return { kind: "locked", stale: true, message: "The vault is locked." };
}

export async function call(command, args) {
  const started = generation;
  let result;
  try {
    result = await invoke(command, args);
  } catch (error) {
    if (started !== generation) throw stale();
    if (error && typeof error === "object" && error.kind === "locked") {
      onLocked();
    }
    throw error;
  }
  if (started !== generation) throw stale();
  return result;
}

/** A human sentence for an error from the backend or the dialog plugin. */
export function describe(error) {
  if (error && typeof error === "object" && "message" in error) {
    if (error.kind === "another_instance") {
      return "Another Asset Manager window already has this vault open.";
    }
    if (error.kind === "locked") return "The vault is locked.";
    return sentence(error.message);
  }
  return sentence(String(error));
}

function sentence(text) {
  if (!text) return "Something went wrong.";
  const first = text.charAt(0).toUpperCase() + text.slice(1);
  return /[.!?]$/.test(first) ? first : first + ".";
}
