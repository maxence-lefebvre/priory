// The connect form and its sign-in, shared by the docked panel and the browser's side panel.
import { address, call } from "./request.js";
import { put } from "./store.js";

/** The form's empty state: what the person typed, the last refusal and whether a check runs. */
export const blank = () => ({ address: "", token: "", error: null, busy: false });

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

/**
 * The form's elements, built with `h` and `icon`. `done` gets the config after a sign-in.
 * `guide` adds the docked panel's note that the Guide tab needs no sign-in.
 */
export function connectForm(c, { h, icon, render, permissions, channel, fetcher, done, guide = false }) {
  const submit = (event) => {
    event.preventDefault();
    const checked = address(c.address);
    if (checked.error) {
      c.error = checked.error;
      return render();
    }
    // Asked inside the click, which the permission prompt needs.
    return permissions.request({ origins: [`${checked.origin}/*`] })
      .then((granted) => connect(c, checked.origin, granted, { fetcher, channel, render }))
      .then((config) => config && done(config));
  };
  return [
    h("div", { class: "q" }, "Connect to your factory"),
    h("div", { class: "read" }, "The extension reads and answers through your daemon's serve address. Your browser asks once to let it reach that address."),
    h("form", { class: "sect", onsubmit: submit },
      h("div", { class: "field" }, h("label", { for: "address" }, "Address"),
        h("input", { id: "address", class: "input", autocomplete: "off", placeholder: "https://desktop.tailnet.ts.net", value: c.address, oninput: (event) => { c.address = event.target.value; } })),
      h("div", { class: "field" }, h("label", { for: "token" }, "Token"),
        h("input", { id: "token", class: "input secret", type: "password", autocomplete: "off", value: c.token, oninput: (event) => { c.token = event.target.value; } }),
        h("div", { class: "note" }, icon("key", "s14"), h("span", {}, "The value of PRIORY_TOKEN on the daemon's machine."))),
      h("div", { class: "box" }, h("div", { class: "fact" }, icon("lock", "s14"), h("span", {}, h("b", {}, "Kept by the extension alone. "), "GitHub's pages, their scripts and other extensions cannot read it. It is sent only to this address, in a header."))),
      c.error ? h("div", { class: "notice", role: "alert" }, c.error) : null,
      h("div", { class: "end" }, h("button", { class: "btn primary", type: "submit", disabled: c.busy }, "Connect"))),
    guide ? h("div", { class: "hr" }) : null,
    guide ? h("div", { class: "note" }, icon("book", "s14"), h("span", {}, "The Guide tab works without signing in: it reads the pull request's description from GitHub.")) : null,
  ];
}
