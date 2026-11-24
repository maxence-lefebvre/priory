// Waits on the state's revision; the server answers once it moves, or after 25 s. A moved revision
// reads the listings again. A failed read waits 5 s before the next wait.
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
      // A refresh during the wait may already hold the answered revision.
      if (revision !== held()) moved();
      ok = revision === held() || (await refresh());
    } catch {
      await refresh();
      ok = false;
    }
    if (!ok) await sleep(5000);
  }
}
