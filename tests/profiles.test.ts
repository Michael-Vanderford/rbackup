import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { loadProfiles, type Profile } from "../src/profiles.ts";

const imported: Profile[] = JSON.parse(readFileSync(new URL("../src/imported-profiles.json", import.meta.url), "utf8"));
const data = new Map<string, string>();
const storage = { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); } };
assert.equal(imported.length, 2);
assert.equal(imported[0].repository, "/mnt/backup/job_1784917153088_ynpdxm8mc");
assert.equal(imported[0].sources, "/home/michael\n/mnt/share");
assert.equal(imported[0].excludes.split("\n").length, 39);
assert.equal(imported[1].repository, "sftp://server-pi3/home/michael/backup");
assert.deepEqual(loadProfiles(storage, imported), imported);
assert.deepEqual(loadProfiles(storage, imported), imported, "restarts must not duplicate imports");
const edited = { ...imported[0], name: "Edited name" };
storage.setItem("restic-profiles", JSON.stringify([edited]));
assert.deepEqual(loadProfiles(storage, imported), [edited], "preserve edits and removed profiles");
data.clear();
storage.setItem("restic-profiles", JSON.stringify([edited]));
assert.deepEqual(loadProfiles(storage, imported), [edited, imported[1]], "preserve existing profiles on first migration");
for (const profile of imported) {
  assert.deepEqual(Object.keys(profile).sort(), ["id", "name", "repository", "passwordFile", "sources", "excludes", "tags"].sort());
  assert.equal(profile.passwordFile, "");
}
data.clear();
storage.setItem("restic-import-docs-jobs-v1", "done");
const legacy = { ...imported[0], repository: "/mnt/backup", passwordFile: "/private/password" };
storage.setItem("restic-profiles", JSON.stringify([legacy]));
const corrected = { ...legacy, repository: imported[0].repository };
assert.deepEqual(loadProfiles(storage, imported), [corrected], "correct already imported profiles while preserving credentials");
assert.deepEqual(JSON.parse(data.get("restic-profiles")!), [corrected], "persist corrected path");
storage.setItem("restic-profiles", JSON.stringify([legacy]));
assert.deepEqual(loadProfiles(storage, imported), [legacy], "do not repeat the migration after a later edit");
data.delete("restic-full-backup-path-v1");
const custom = { ...legacy, repository: "/custom/repository" };
storage.setItem("restic-profiles", JSON.stringify([custom]));
assert.deepEqual(loadProfiles(storage, imported), [custom], "preserve custom paths");
console.log("Profile import checks passed.");
