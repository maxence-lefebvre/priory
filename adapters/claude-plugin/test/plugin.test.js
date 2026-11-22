import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";

const root = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");
const json = (path) => JSON.parse(read(path));

// The verbs only a person runs, read from the guard itself so the two never drift.
const guard = readFileSync(new URL("../../../crates/priory-factory/src/guard.rs", import.meta.url), "utf8");
const personVerbs = [
  ...guard.match(/pub const PERSON_VERBS: &\[&str\] = &\[([^\]]*)\]/)[1].matchAll(/"([a-z-]+)"/g),
].map((match) => match[1]).concat(["report pick", "memories accept", "memories drop", "batch edit"]);

const skills = readdirSync(new URL("skills/", root)).map((name) => `skills/${name}/SKILL.md`);
const commands = readdirSync(new URL("commands/", root)).map((name) => `commands/${name}`);

/** The YAML frontmatter of a skill or command file as key-value pairs, and its body. */
function frontmatter(path) {
  const text = read(path);
  const found = text.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
  assert.ok(found, `${path} has no frontmatter`);
  const fields = Object.fromEntries(
    found[1].split("\n").map((line) => line.match(/^([a-z-]+): (.*)$/)).filter(Boolean).map((m) => [m[1], m[2]]),
  );
  return { fields, body: found[2] };
}

test("the manifest names the plugin priory, so its skills and commands are /priory:...", () => {
  const manifest = json(".claude-plugin/plugin.json");
  assert.equal(manifest.name, "priory");
  assert.equal(typeof manifest.description, "string");
});

test("the plugin starts priory mcp --outside and runs the outside hook before each Bash call", () => {
  assert.deepEqual(json(".mcp.json"), { mcpServers: { priory: { command: "priory", args: ["mcp", "--outside"] } } });
  assert.deepEqual(json("hooks/hooks.json"), {
    hooks: {
      PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "priory hook PreToolUse --outside" }] }],
    },
  });
});

test("the plugin has the desk, ask and add skills and the answer command, each with its frontmatter", () => {
  assert.deepEqual(skills.sort(), ["skills/add/SKILL.md", "skills/ask/SKILL.md", "skills/desk/SKILL.md"]);
  assert.deepEqual(commands, ["commands/answer.md"]);
  for (const path of [...skills, ...commands]) {
    const { fields, body } = frontmatter(path);
    assert.ok(fields.description?.length > 20, `${path} needs a description`);
    assert.ok(body.trim().length > 0, `${path} has no body`);
  }
  for (const path of skills) {
    assert.equal(frontmatter(path).fields.name, path.split("/")[1]);
  }
});

test("only the person can invoke /priory:answer, and it grants no tool permission", () => {
  const { fields, body } = frontmatter("commands/answer.md");
  assert.equal(fields["disable-model-invocation"], "true");
  assert.equal(fields["allowed-tools"], undefined);
  // It prints the line; it never runs one before the model sees it.
  assert.doesNotMatch(body, /!`/);
  assert.match(body, /! priory answer --revision/);
});

test("no skill or command tells the model to run a verb only a person runs", () => {
  for (const path of [...skills, ...commands]) {
    for (const line of read(path).split("\n")) {
      for (const verb of personVerbs) {
        for (const at of line.matchAll(new RegExp(`priory ${verb}\\b`, "g"))) {
          // A line for the person to run carries Claude Code's `!` prefix, and only the answer command prints one.
          const forThePerson = line.slice(0, at.index).endsWith("! ") && path === "commands/answer.md";
          assert.ok(forThePerson, `${path}: "${line.trim()}" names \`priory ${verb}\` as an instruction`);
        }
      }
    }
  }
});

test("each skill calls only the tools the outside server serves", () => {
  const served = ["desk", "asks", "task", "ask", "add"];
  for (const path of [...skills, ...commands]) {
    for (const match of read(path).matchAll(/the `([a-z]+)` tool/g)) {
      assert.ok(served.includes(match[1]), `${path} names the \`${match[1]}\` tool`);
    }
  }
  assert.ok(existsSync(new URL("README.md", root)));
});
