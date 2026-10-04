// URLs for vault media, served by the backend's asset:// handler, which
// decrypts in-process and answers nothing while the vault is locked.
//
// Tauri exposes a custom scheme as `asset://localhost/` on Linux and macOS
// but as `http://asset.localhost/` on Windows; the CSP allows both.

const BASE = navigator.userAgent.includes("Windows") ? "http://asset.localhost" : "asset://localhost";

/** variant: 256 or 1024 for a thumbnail, omitted for the original. */
export function mediaUrl(objectId, variant) {
  return `${BASE}/media/${objectId}${variant ? `?variant=${variant}` : ""}`;
}
