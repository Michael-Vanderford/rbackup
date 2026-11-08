export type Maintenance = { keepLast: number; keepDaily: number; keepWeekly: number; keepMonthly: number; readData: boolean };

/** Persisted profile metadata: id, name, repository, passwordFile, sources, excludes, and tags are strings. Multiline source, exclude, and tag fields are normalized when running commands. Optional maintenance settings control the post-backup integrity check and retention policy. Password text is never part of this type. */
export type Profile = {
  id: string;
  name: string;
  repository: string;
  passwordFile: string;
  sources: string;
  excludes: string;
  tags: string;
  maintenance?: Maintenance;
};

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">;
const importKey = "restic-import-docs-jobs-v1";

/** Read and validate the saved profile array, correct the legacy Full Backup path once, then merge bundled profiles once by ID. Persist the merged array and import marker. Existing IDs win; later profile removals stay removed. Throws on invalid saved JSON, invalid profile fields, or storage failures. */
export function loadProfiles(storage: Storage, imported: Profile[]): Profile[] {
  const saved: unknown = JSON.parse(storage.getItem("restic-profiles") || "[]");
  if (!Array.isArray(saved) || !saved.every(p => p && ["id", "name", "repository", "passwordFile", "sources", "excludes", "tags"].every(k => typeof p[k] === "string"))) {
    throw new Error("Saved repository profiles are invalid.");
  }
  let profiles = saved as Profile[];
  // Correct the original Full Backup import once, preserving custom paths and removals.
  const pathFixKey = "restic-full-backup-path-v1";
  if (!storage.getItem(pathFixKey)) {
    profiles = profiles.map(p => p.id === "job_1784917153088_ynpdxm8mc" && p.repository === "/mnt/backup"
      ? { ...p, repository: "/mnt/backup/job_1784917153088_ynpdxm8mc" } : p);
    storage.setItem("restic-profiles", JSON.stringify(profiles));
    storage.setItem(pathFixKey, "done");
  }
  if (storage.getItem(importKey)) return profiles;

  // Import once, preserving existing settings and deliberate later profile removals.
  const existing = new Set(profiles.map(p => p.id));
  const merged = [...profiles, ...imported.filter(p => !existing.has(p.id))];
  storage.setItem("restic-profiles", JSON.stringify(merged));
  storage.setItem(importKey, "done");
  return merged;
}
