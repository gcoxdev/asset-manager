// The IPC boundary.
//
// Every call goes through call(), so a "locked" rejection from the backend
// — the vault auto-locked, or was locked from another code path — sends the
// UI to the unlock screen instead of surfacing as a confusing error in
// whatever view happened to be open.

import { invoke } from "@tauri-apps/api/core";

let onLocked = () => {};

export function setLockedHandler(fn) {
  onLocked = fn;
}

export async function call(command, args) {
  try {
    return await invoke(command, args);
  } catch (error) {
    if (error && typeof error === "object" && error.kind === "locked") {
      onLocked();
    }
    throw error;
  }
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
