// Waits on the state's revision; the server answers once it moves, or after 25 s. A moved revision
// reads the mounted queries again. A failed read waits 5 s before the next wait. Nothing reads while
// `ready` has no revision: signed out, hidden, or before the first read.
export async function follow({ live, ready, held, wait, moved, refresh, sleep }) {
  while (live()) {
    const since = ready();
    if (since === undefined) {
      await sleep(1000);
      continue;
    }
    let ok;
    try {
      const { revision } = await wait(since);
      // The tab may have hidden or signed out during the wait.
      if (ready() === undefined) continue;
      // A refresh during the wait may already hold the answered revision.
      if (revision !== held()) moved();
      ok = revision === held() || (await refresh());
    } catch {
      if (ready() === undefined) continue;
      await refresh();
      ok = false;
    }
    if (!ok) await sleep(5000);
  }
}
