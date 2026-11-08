import assert from "node:assert/strict";
import { loadProfiles, type Profile } from "../src/profiles.ts";

const data = new Map<string, string>();
const storage = { getItem: (key: string) => data.get(key) ?? null };
assert.deepEqual(loadProfiles(storage), [], "first launch must not import personal jobs");
assert.equal(data.size, 0, "loading must not write migration or import settings");
const profile: Profile = { id: "example", name: "My repository", repository: "/backup", passwordFile: "/private/password", sources: "/documents", excludes: "*.tmp", tags: "daily", maintenance: { keepLast: 3, keepDaily: 7, keepWeekly: 4, keepMonthly: 6, readData: false } };
data.set("restic-profiles", JSON.stringify([profile]));
assert.deepEqual(loadProfiles(storage), [profile], "saved jobs and maintenance settings must survive restart");
assert.deepEqual(loadProfiles(storage), [profile], "repeated loading must not add jobs");
data.set("restic-profiles", "[]");
data.set("restic-import-docs-jobs-v1", "done");
assert.deepEqual(loadProfiles(storage), [], "old migration markers must not resurrect removed jobs");
for (const saved of ["null", "{}", "[{}]", "invalid JSON"]) {
  data.set("restic-profiles", saved);
  assert.throws(() => loadProfiles(storage), "invalid storage must be rejected");
}
console.log("Clean-start and saved-profile checks passed.");
