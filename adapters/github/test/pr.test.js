import { test } from "node:test";
import assert from "node:assert/strict";
import { headSha, parsePr } from "../lib/pr.js";

test("a pull request page names its repository, number, resource and tab", () => {
  assert.deepEqual(parsePr("https://github.com/mlfbvr/priory/pull/164"), {
    repo: "mlfbvr/priory",
    number: 164,
    resource: "github:mlfbvr/priory#164",
    tab: "conversation",
    url: "https://github.com/mlfbvr/priory/pull/164",
  });
  assert.equal(parsePr("https://github.com/mlfbvr/priory/pull/164/files?w=1#diff-1").tab, "files");
  assert.equal(parsePr("https://github.com/o.x/r-y_z/pull/7/commits").tab, "commits");
});

test("a page that is not a pull request on github.com names none", () => {
  for (const url of ["https://github.com/mlfbvr/priory", "https://github.com/mlfbvr/priory/issues/3", "https://evil.example/o/r/pull/1", "https://github.com/o/r/pull/x", "not a url"]) {
    assert.equal(parsePr(url), null, url);
  }
});

test("the head sha is read from the page's embedded pull request data", () => {
  const sha = "cc36d32a212a2b8b6611fb73549fe6d04fb6ec38";
  assert.equal(headSha(`{"headBranch":"b","headSha":"${sha}","id":1}`), sha);
  assert.equal(headSha("<html>no data</html>"), null);
});
