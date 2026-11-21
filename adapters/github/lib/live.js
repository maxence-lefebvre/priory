// One /api/events poller per browser, by Web Lock, sharing revisions on a BroadcastChannel: serve holds 4 waiters.
import { call } from "./request.js";
import { get } from "./store.js";

const channel = new BroadcastChannel("priory-events");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Calls `handler` with each { revision } or { unreachable } any frame's poller sees. */
export function live(handler) {
  channel.addEventListener("message", (event) => handler(event.data));
  navigator.locks.request("priory-events", async () => {
    let revision = -1;
    let backoff = 5000;
    let failed = false;
    for (;;) {
      const config = await get("config");
      if (!config) {
        await sleep(2000);
        continue;
      }
      const reply = await call(fetch, config.origin, config.token, `/api/events?since=${revision}`, undefined, 35000);
      const share = (message) => {
        channel.postMessage(message);
        handler(message);
      };
      if (reply.status === 200) {
        // After a failure, frames show "unreachable" until told otherwise, even at the same revision.
        if (reply.body.revision !== revision || failed) share({ revision: (revision = reply.body.revision) });
        backoff = 5000;
        failed = false;
        continue;
      }
      if (reply.status === 503) {
        await sleep(1000);
        continue;
      }
      failed = true;
      share(reply.status === 401 ? { signedOut: true } : { unreachable: true });
      await sleep(backoff);
      backoff = Math.min(backoff * 2, 60000);
    }
  });
}
