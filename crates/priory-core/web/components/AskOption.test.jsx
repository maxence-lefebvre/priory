import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { AskOption } from "./AskOption.jsx";

const draw = (props) => renderToStaticMarkup(<AskOption option="Merge" index={0} {...props} />);

test("an idle option names itself and says why it stands out", () => {
  const html = draw({ recommended: true, runs: true });
  assert.match(html, /class="opt rec-opt  "/);
  assert.match(html, /aria-disabled="false"/);
  assert.match(html, /<span>Merge<small>Recommended\. Runs a command<\/small><\/span>/);
  assert.doesNotMatch(html, /aria-busy|busy/);
});

test("the pressed option shows its answer sending on itself, its label kept", () => {
  const html = draw({ recommended: true, sending: true, off: true });
  assert.match(html, /class="opt rec-opt chosen "/);
  assert.match(html, /aria-busy="true"/);
  assert.match(html, /<span class="busy" aria-hidden="true"><\/span><span>Merge<small>Sending…<\/small><\/span>/);
});

test("the other options take no press while one sends", () => {
  const html = draw({ off: true });
  assert.match(html, /aria-disabled="true"/);
  assert.doesNotMatch(html, /aria-busy/);
});

test("an armed option takes the host's label or note, and its key hint", () => {
  assert.match(draw({ armed: true, note: "Press again to send", hint: 1 }), /class="opt   armed".*<small>Press again to send<\/small><\/span><kbd>1<\/kbd>/);
  assert.match(draw({ armed: true, label: <b>Confirm: Merge</b> }), /<b>Confirm: Merge<\/b><\/button>/);
});
