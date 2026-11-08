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

type Storage = Pick<globalThis.Storage, "getItem">;

/** Read saved profiles without importing defaults or changing user settings. Empty storage returns no profiles. */
export function loadProfiles(storage: Storage): Profile[] {
  const saved: unknown = JSON.parse(storage.getItem("restic-profiles") || "[]");
  if (!Array.isArray(saved) || !saved.every(p => p && ["id", "name", "repository", "passwordFile", "sources", "excludes", "tags"].every(k => typeof p[k] === "string"))) {
    throw new Error("Saved repository profiles are invalid.");
  }
  return saved as Profile[];
}
