// The connect form and its sign-in, shared by the docked panel and the browser's side panel.
import { address, call } from "./request.js";
import { put } from "./store.js";
import { Icon } from "../../../crates/priory-core/web/components/Icon.jsx";

/** The form's empty state: what the person typed, the last refusal and whether a check runs. */
const blank = () => ({ address: "", token: "", error: null, busy: false });

/**
 * Checks the token against `origin`, then stores it and tells every other frame on `channel`.
 * Returns the config, or null with `c.error` set.
 */
export async function connect(c, origin, granted, { fetcher = fetch, channel, render }) {
  if (!granted) {
    c.error = `The browser did not let the extension reach ${origin}.`;
    render();
    return null;
  }
  c.busy = true;
  render();
  const checked = await call(fetcher, origin, c.token.trim(), "/api/views");
  c.busy = false;
  if (checked.status !== 200) {
    c.error = checked.status === 401 ? "The daemon refused this token. Paste the current one." : `Could not reach priory serve at ${origin}. Check the address and that it runs.`;
    render();
    return null;
  }
  const config = { origin, token: c.token.trim() };
  await put("config", config);
  Object.assign(c, blank());
  channel.postMessage({ signedIn: true });
  return config;
}

// What the person typed in this frame's one connect form, the last refusal and whether a check runs.
const form = blank();

/**
 * The connect form, which keeps what the person types and calls `render` to draw it again.
 * `done` gets the config after a sign-in. `guide` adds the docked panel's note that the Guide tab needs no sign-in.
 */
export function ConnectForm({ render, permissions, channel, fetcher, done, guide = false }) {
  const submit = (event) => {
    event.preventDefault();
    const checked = address(form.address);
    if (checked.error) {
      form.error = checked.error;
      return render();
    }
    // Asked inside the click, which the permission prompt needs.
    return permissions.request({ origins: [`${checked.origin}/*`] })
      .then((granted) => connect(form, checked.origin, granted, { fetcher, channel, render }))
      .then((config) => config && done(config));
  };
  const typed = (key) => (event) => {
    form[key] = event.target.value;
    render();
  };
  return (
    <>
      <div className="q">Connect to your factory</div>
      <div className="read">The extension reads and answers through your daemon's serve address. Your browser asks once to let it reach that address.</div>
      <form className="sect" onSubmit={submit}>
        <div className="field"><label htmlFor="address">Address</label>
          <input id="address" className="input" autoComplete="off" placeholder="https://desktop.tailnet.ts.net" value={form.address} onChange={typed("address")} /></div>
        <div className="field"><label htmlFor="token">Token</label>
          <input id="token" className="input secret" type="password" autoComplete="off" value={form.token} onChange={typed("token")} />
          <div className="note"><Icon name="key" size="s14" /><span>The value of PRIORY_TOKEN on the daemon's machine.</span></div></div>
        <div className="box"><div className="fact"><Icon name="lock" size="s14" /><span><b>Kept by the extension alone. </b>GitHub's pages, their scripts and other extensions cannot read it. It is sent only to this address, in a header.</span></div></div>
        {form.error ? <div className="notice" role="alert">{form.error}</div> : null}
        <div className="end"><button className="btn primary" type="submit" disabled={form.busy}>Connect</button></div>
      </form>
      {guide ? <><div className="hr" /><div className="note"><Icon name="book-open" size="s14" /><span>The Guide tab works without signing in: it reads the pull request's description from GitHub.</span></div></> : null}
    </>
  );
}
