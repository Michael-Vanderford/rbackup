import assert from "node:assert/strict";
import { toggleSelection, within } from "../src/treeSelection.ts";
const children = new Map([
  ["/", [{ path: "/home" }, { path: "/etc" }]],
  ["/home", [{ path: "/home/alex" }, { path: "/home/other" }]],
  ["/home/alex", [{ path: "/home/alex/a.txt" }, { path: "/home/alex/b.txt" }]],
]);
let selected = toggleSelection(new Set(), "/", children);
assert.deepEqual([...selected], ["/"]);
selected = toggleSelection(selected, "/home/alex/a.txt", children);
assert.deepEqual([...selected].sort(), ["/etc", "/home/alex/b.txt", "/home/other"]);
assert(![...selected].some(parent => within("/home/alex/a.txt", parent)));
selected = toggleSelection(selected, "/home", children);
assert.deepEqual([...selected].sort(), ["/etc", "/home"]);
selected = toggleSelection(selected, "/home", children);
assert.deepEqual([...selected], ["/etc"]);
assert(!within("/homework", "/home"));
assert(within("/home/alex", "/home"));
console.log("Tree checkbox selection checks passed.");
