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

/** Build a Restic location for an externally configured rclone remote. */
export function rcloneRepository(remote: string, path: string): string {
  const name = remote.trim();
  if (!/^[\p{L}\p{N}_][\p{L}\p{N}_. -]*$/u.test(name)) return "";
  return `rclone:${name}:${path}`;
}

/** Build a path-style Restic S3 location without embedded credentials. */
export function s3Repository(endpoint: string, bucket: string, prefix: string): string {
  const server = endpoint.trim();
  const name = bucket.trim();
  if (!/^https?:\/\/[^/\\?#@\s]+\/?$/i.test(server)) return "";
  if (!/^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(name) || name.includes("..") || /^\d+\.\d+\.\d+\.\d+$/.test(name)) return "";

  const segments = prefix.trim().split("/").filter(Boolean);
  if (segments.some(segment => segment === "." || segment === "..")) return "";
  try {
    const url = new URL(server);
    if (/\.s3[.-].*amazonaws\.com(?:\.cn)?$/.test(url.hostname)) return "";
    return `s3:${url.origin}/${[name, ...segments.map(segment => encodeURIComponent(segment))].join("/")}`;
  } catch {
    return "";
  }
}
