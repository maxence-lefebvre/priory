// One /api/events poller per browser, by Web Lock, sharing revisions on a BroadcastChannel. A hidden
// frame polls nothing: it gives the lock back, so a shown frame of another tab takes it.
import { call } from "./request.js";
import { get } from "./store.js";

const channel = new BroadcastChannel("priory-events");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Whether this frame's tab is hidden. */
export const hidden = () => globalThis.document?.visibilityState === "hidden";

/** Resolves once this frame's tab shows. */
export function shown() {
  if (!hidden()) return Promise.resolve();
  return new Promise((resolve) => {
    const on = () => {
      if (hidden()) return;
      document.removeEventListener("visibilitychange", on);
      resolve();
    };
    document.addEventListener("visibilitychange", on);
  });
}

/**
 * How long to wait before the next call after a reply of `status`, given the wait before it: none
 * after a 200, else twice as long, from 1 s for a busy server's 503 and 5 s for any other failure,
 * up to 60 s, with up to a fifth more at random so tabs do not call together.
 */
export function backoff(status, before, random = Math.random) {
  if (status === 200) return 0;
  const doubled = Math.min(Math.max(before * 2, status === 503 ? 1000 : 5000), 60000);
  return Math.round(doubled * (1 + random() / 5));
}

/** Calls `handler` with each { revision } or { unreachable } any frame's poller sees. */
export function live(handler) {
  channel.addEventListener("message", (event) => handler(event.data));
  const share = (message) => {
    channel.postMessage(message);
    handler(message);
  };
  let revision = -1;
  let wait = 0;
  let failed = false;
  const poll = async () => {
    while (!hidden()) {
      const config = await get("config");
      if (!config) {
        await sleep(2000);
        continue;
      }
      const reply = await call(fetch, config.origin, config.token, `/api/events?since=${revision}`, undefined, 35000);
      wait = backoff(reply.status, wait);
      if (reply.status === 200) {
        // After a failure, frames show "unreachable" until told otherwise, even at the same revision.
        if (reply.body.revision !== revision || failed) share({ revision: (revision = reply.body.revision) });
        failed = false;
        continue;
      }
      if (reply.status !== 503) {
        failed = true;
        share(reply.status === 401 ? { signedOut: true } : { unreachable: true });
      }
      await sleep(wait);
    }
  };
  (async () => {
    for (;;) {
      await shown();
      await navigator.locks.request("priory-events", poll);
    }
  })();
}
