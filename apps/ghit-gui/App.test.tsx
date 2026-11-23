import { expect, test } from "nsh/test";
import { parseDiff, relTime, step } from "./gh";
import { buildGraph, tick } from "./graph";
import { rowStatus } from "./icons";

const DIFF = [
  "diff --git a/src/x.rs b/src/x.rs",
  "index 1..2 100644",
  "--- a/src/x.rs",
  "+++ b/src/x.rs",
  "@@ -10,3 +10,3 @@ fn main() {",
  " keep",
  "-old",
  "+new",
  "\\ No newline at end of file",
  "",
].join("\n");

test("parses a diff into rows with line numbers", () => {
  const rows = parseDiff(DIFF);
  expect(rows.map((r) => r.kind)).toEqual(["file", "hunk", "ctx", "del", "add"]);
  expect(rows[0].text).toBe("src/x.rs");
  expect(rows[1].text).toBe("fn main() {");
  expect([rows[2].old, rows[2].new]).toEqual([10, 10]);
  expect(rows[3].old).toBe(11);
  expect(rows[4].new).toBe(11);
});

test("formats relative time", () => {
  const now = Date.parse("2026-01-10T00:00:00Z");
  expect(relTime("2026-01-09T23:59:30Z", now)).toBe("30s");
  expect(relTime("2026-01-09T22:00:00Z", now)).toBe("2h");
  expect(relTime("2026-01-07T00:00:00Z", now)).toBe("3d");
});

test("clamps selection steps", () => {
  expect(step(0, -1, 5)).toBe(0);
  expect(step(3, 10, 5)).toBe(4);
  expect(step(-1, 1, 0)).toBe(-1);
});

test("maps states to statuses", () => {
  expect(rowStatus("issue", "CLOSED", false, "NOT_PLANNED")).toBe("shut");
  expect(rowStatus("issue", "CLOSED", false, "COMPLETED")).toBe("done");
  expect(rowStatus("pr", "CLOSED")).toBe("shut");
  expect(rowStatus("pr", "OPEN", true)).toBe("draft");
});

test("builds edges in their direction and drops outside links", () => {
  const g = buildGraph({
    issues: [
      { number: 1, title: "a", state: "OPEN", stateReason: null, parent: null, blockedBy: { nodes: [{ number: 2 }, { number: 99 }] } },
      { number: 2, title: "b", state: "CLOSED", stateReason: "COMPLETED", parent: { number: 1 }, blockedBy: null },
    ],
    prs: [{ number: 3, title: "c", state: "MERGED", isDraft: false, closingIssuesReferences: { nodes: [{ number: 1 }] } }],
  });
  expect(g.edges.map((e) => [g.nodes[e.src].id, g.nodes[e.dst].id, e.kind])).toEqual([
    [2, 1, "blocks"],
    [1, 2, "parent"],
    [3, 1, "closes"],
  ]);
  expect(g.nodes.map((n) => n.status)).toEqual(["open", "done", "done"]);
});

test("pulls linked nodes together and cools", () => {
  const g = buildGraph({
    issues: [
      { number: 1, title: "a", state: "OPEN", stateReason: null, parent: null, blockedBy: null },
      { number: 2, title: "b", state: "OPEN", stateReason: null, parent: { number: 1 }, blockedBy: null },
    ],
    prs: [],
  });
  g.nodes[1].x = 500;
  let alpha = 1;
  for (let i = 0; i < 300; i++) alpha = tick(g, [true, true], alpha);
  const d = Math.hypot(g.nodes[0].x - g.nodes[1].x, g.nodes[0].y - g.nodes[1].y);
  expect(d < 120).toBe(true);
  expect(alpha < 0.01).toBe(true);
});
