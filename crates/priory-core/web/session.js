// This device's sign-in: the token, the requests that carry it, and whether the last read failed.
import { store } from "./store.js";

const kept = typeof localStorage === "undefined" ? null : localStorage.getItem("priory.token");
export const session = store({ token: kept, signInError: null, failed: null });
const change = (part) => session.set({ ...session.get(), ...part });

// The kept listings. An earlier version of the web view reads and writes the other key in its own shape.
export const SNAPSHOT = "priory.snapshot.2";
export const EARLIER_SNAPSHOT = "priory.snapshot";

// The service worker reads the same copy when the push service replaces the subscription.
export const KEPT_KINDS = "/api/worker/kinds";

// The service worker cannot read local storage, so it gets its own copy of the token to tell the
// daemon about a subscription the push service replaced. Signing out drops it, and the kept kinds.
export function keepTokenForWorker(token) {
  if (!("caches" in globalThis)) return;
  caches.open("priory-push")
    .then((cache) => token
      ? cache.put("/api/worker/token", new Response(token))
      : Promise.all(["/api/worker/token", KEPT_KINDS].map((key) => cache.delete(key))))
    .catch(() => {});
}

export async function api(path, signal) {
  const { token } = session.get();
  const response = await fetch(path, { headers: { Authorization: `Bearer ${token}` }, cache: "no-store", signal });
  if (response.status === 401) throw Object.assign(new Error("the token was refused"), { status: 401, token });
  if (!response.ok) throw Object.assign(new Error(`${path} answered ${response.status}`), { status: response.status });
  return response.json();
}

export async function post(path, body) {
  const response = await fetch(path, {
    method: "POST",
    headers: { Authorization: `Bearer ${session.get().token}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
    cache: "no-store",
  });
  return { status: response.status, body: await response.json().catch(() => ({})) };
}

export function setFailed(failed) {
  if (session.get().failed !== failed) change({ failed });
}

export const failureWords = (error) => (error instanceof TypeError ? "the connection dropped" : error.message);

export function signIn(token) {
  localStorage.setItem("priory.token", token);
  keepTokenForWorker(token);
  change({ token, signInError: null });
}

const leaving = new Set();
// What else forgets this device's data on a sign-out. It runs while the token still works.
export const onSignOut = (forget) => leaving.add(forget);

export function signOut(reason) {
  for (const key of ["priory.token", SNAPSHOT, EARLIER_SNAPSHOT]) localStorage.removeItem(key);
  keepTokenForWorker(null);
  for (const forget of leaving) forget();
  session.set({ token: null, signInError: reason, failed: null });
}

export const refuse = () => signOut("The daemon refused this token. Paste the current one.");

// Why a write did not go; `refused` words a refusal the daemon gave.
export function writeRefusal(status, body, refused = () => `Not sent: ${body.error || `the server answered ${status}`}.`) {
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the report before you send again.";
  return refused();
}
