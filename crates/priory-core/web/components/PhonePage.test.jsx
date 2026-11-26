import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { Panel } from "./PhonePage.jsx";

test("a panel keeps a class of its own", () => {
  assert.equal(renderToStaticMarkup(<Panel label="Reports" kind="report" />), '<div class="panel report" role="region" aria-label="Reports"></div>');
});
