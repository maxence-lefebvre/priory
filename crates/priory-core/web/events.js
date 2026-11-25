// Waits on the state's revision; the server answers once it moves, or after 25 s. A moved revision
// reads the mounted queries again. A failed read, or one that left the held revision where it was, waits
// 5 s before the next wait. Nothing reads while `ready` has no revision: signed out, hidden, or before
// the first read.
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
      ok = revision === held() || ((await refresh()) && !(held() <= since));
    } catch {
      if (ready() === undefined) continue;
      await refresh();
      ok = false;
    }
    if (!ok) await sleep(5000);
  }
}

// A `stuck` page has no revision to wait at, and a wait of the page before may still be out: the page
// reads again after 5 s. Nothing reads unless `may`: the person is signed in and the tab shown.
export async function recover({ live, may, stuck, refresh, sleep }) {
  while (live()) {
    const was = may() && stuck();
    await sleep(was ? 5000 : 1000);
    if (was && may() && stuck()) await refresh();
  }
}
