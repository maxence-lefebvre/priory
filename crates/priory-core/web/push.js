// The Notifications screen's pure parts: the kinds a device chooses, and how a choice reaches the daemon.

/** Each kind, its name and the sentence under it. Every value is a constant, so the markup needs no escaping. */
export const PUSH_KINDS = [
  ["ask", "Asks", "An agent asks you something."],
  ["stopped", "Stopped tasks", "A task is blocked or failed."],
  ["intake", "Intake failures", "A source cannot be polled."],
  ["uncertain", "Uncertain work", "Priory cannot tell whether a prompt or an answer command ran."],
  ["cap", "Spend caps", "A day or week cap is reached."],
  ["landed", "Landed wrong", "A merged change turned red or was reverted."],
  ["stall", "Queue stalls", "No task can start until the machine has room."],
];

/**
 * The checkboxes of the kinds the daemon replied with, as markup; nothing for a daemon that has none.
 * @param {Record<string, boolean> | null} kinds @param {boolean} busy
 */
export function kindBoxes(kinds, busy) {
  if (!kinds) return "";
  const rows = PUSH_KINDS.filter(([kind]) => kind in kinds);
  const boxes = rows.map(([kind, name, line]) => `<label class="push-kind"><input type="checkbox" id="push-kind-${kind}" data-push-kind="${kind}"${kinds[kind] ? " checked" : ""} aria-disabled="${busy}"><span><b>${name}</b><span>${line}</span></span></label>`);
  const none = rows.some(([kind]) => kinds[kind]) ? "" : '<p class="note" role="status">Nothing is chosen, so this device gets no notification.</p>';
  return `<fieldset class="push-kinds"><legend>Notify this device about</legend>${boxes.join("")}</fieldset>${none}<p class="note">Each device keeps its own choice. A change applies at once.</p>`;
}

/**
 * Sends a subscription as every version of the daemon takes it, then again with the kept choice when
 * the daemon's differs, such as after it lost its state. The device's own choice wins.
 * @param {any} subscription
 * @param {{ send: (body: any) => Promise<{ status: number, body: any }>, kept: () => Promise<Record<string, boolean> | null> }} io
 */
export async function confirmPush(subscription, { send, kept }) {
  const reply = await send(subscription);
  const kinds = reply.status === 200 ? reply.body.kinds ?? null : null;
  const choice = kinds && (await kept().catch(() => null));
  const known = Object.entries(choice || {}).filter(([kind]) => kinds && kind in kinds);
  if (!known.some(([kind, on]) => kinds[kind] !== on)) return { ...reply, kinds };
  const again = await send({ ...subscription, kinds: Object.fromEntries(known) }).catch(() => ({ status: 0, body: {} }));
  return { ...reply, kinds: again.status === 200 ? again.body.kinds : kinds };
}

/**
 * Keeps then sends a changed choice, and returns the daemon's. A refusal or a dropped connection puts
 * the kept choice back, so it never differs from what the screen shows.
 * @param {any} subscription @param {Record<string, boolean>} kinds
 * @param {{ send: (body: any) => Promise<{ status: number, body: any }>, kept: () => Promise<Record<string, boolean> | null>, keep: (kinds: Record<string, boolean> | null) => Promise<unknown> }} io
 */
export async function saveKinds(subscription, kinds, { send, kept, keep }) {
  const before = await kept();
  await keep(kinds);
  try {
    const { status, body } = await send({ ...subscription, kinds });
    if (status !== 200) throw new Error(body.error || `the server answered ${status}`);
    return body.kinds;
  } catch (error) {
    await keep(before);
    throw error;
  }
}
