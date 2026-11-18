import assert from "node:assert/strict";
import { loadProfiles, rcloneRepository, s3Repository, type Profile } from "../src/profiles.ts";

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
assert.equal(rcloneRepository(" Work Drive ", "bucket/Backup files"), "rclone:Work Drive:bucket/Backup files");
assert.equal(rcloneRepository("cloud", ""), "rclone:cloud:");
assert.equal(rcloneRepository("cloud", "/backups"), "rclone:cloud:/backups");
for (const remote of ["", "cloud:other", "cloud,type=local", "--flag", ":local"]) {
  assert.equal(rcloneRepository(remote, "backup"), "");
}
assert.equal(s3Repository("https://s3.us-east-1.amazonaws.com", "my-backups", ""), "s3:https://s3.us-east-1.amazonaws.com/my-backups");
assert.equal(s3Repository("http://localhost:9000/", "my-backups", "/folder/Backup #1/"), "s3:http://localhost:9000/my-backups/folder/Backup%20%231");
for (const endpoint of ["https://user:secret@example.test", "https://example.test/bucket", "https://example.test?key=secret", "https://bucket.s3.us-east-1.amazonaws.com", "ftp://example.test"]) {
  assert.equal(s3Repository(endpoint, "my-backups", ""), "");
}
assert.equal(s3Repository("https://example.test", "bad/bucket", ""), "");
assert.equal(s3Repository("https://example.test", "my-backups", "folder/../other"), "");
console.log("Clean-start, saved-profile, and repository location checks passed.");
