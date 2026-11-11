import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./App.css";
import { loadProfiles, type Profile } from "./profiles";
import SchedulePanel from "./SchedulePanel";
import { schedulerAction } from "./scheduler";

type Snapshot = { id: string; short_id: string; time: string; hostname: string; paths: string[]; tags?: string[] };
type Result = { stdout: string; stderr: string; code: number };
type Entry = { path: string; type: string; size?: number };
/** Create a new repository profile with a random UUID and empty connection and backup settings. */
const blank = (): Profile => ({ id: crypto.randomUUID(), name: "My repository", repository: "", passwordFile: "", sources: "", excludes: "", tags: "" });
/** Load saved profiles. Return a blank profile if none exist or storage, JSON parsing, or validation fails. */
function load(): Profile[] {
  try {
    const profiles = loadProfiles(localStorage);
    return profiles.length ? profiles : [blank()];
  } catch {
    return [blank()];
  }
}
/** Split multiline source paths, excludes, or tags into trimmed, nonempty strings. */
const lines = (s: string) => s.split("\n").map(s => s.trim()).filter(Boolean);
/** Format a byte count using B, KB, MB, or GB with powers of 1024. */
const size = (n = 0) => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`;

type Theme = "system" | "light" | "dark";

/** Render the desktop workspace and own theme, profiles, connection, operation, snapshot, restore, and retention state. Browser mode previews the interface without executing restic. */
function App() {
  const [theme, setTheme] = useState<Theme>(() => {
    try {
      const saved = localStorage.getItem("restic-theme");
      return saved === "light" || saved === "dark" ? saved : "system";
    } catch { return "system"; }
  });
  useEffect(() => {
    const system = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => { document.documentElement.dataset.theme = theme === "system" ? (system.matches ? "dark" : "light") : theme; };
    apply();
    try { localStorage.setItem("restic-theme", theme); } catch { /* Theme still works without storage. */ }
    system.addEventListener("change", apply);
    return () => system.removeEventListener("change", apply);
  }, [theme]);
  const [profiles, setProfiles] = useState<Profile[]>(load);
  const [active, setActive] = useState(() => {
    try {
      const saved = localStorage.getItem("restic-active-profile");
      return profiles.find(p => p.id === saved)?.id ?? profiles[0].id;
    } catch { return profiles[0].id; }
  });
  useEffect(() => {
    try { localStorage.setItem("restic-active-profile", active); } catch { /* Selection still works without storage. */ }
  }, [active]);
  const profile = profiles.find(p => p.id === active)!;
  const [password, setPassword] = useState("");
  const [tab, setTab] = useState(profile.repository ? "Overview" : "Repository");
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [connected, setConnected] = useState(false);
  const [selected, setSelected] = useState("");
  const restoreHeading = useRef<HTMLHeadingElement>(null);
  const [restoreRequest, setRestoreRequest] = useState(0);
  // Reveal the restore panel only when explicitly opened from snapshot history.
  useEffect(() => {
    if (!restoreRequest) return;
    restoreHeading.current?.focus({ preventScroll: true });
    restoreHeading.current?.scrollIntoView({ behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth", block: "start" });
  }, [restoreRequest]);
  const [files, setFiles] = useState<Entry[]>([]);
  const [browseDirectory, setBrowseDirectory] = useState("/");
  const [filePage, setFilePage] = useState(0);
  const [query, setQuery] = useState("");
  const [target, setTarget] = useState("");
  const [include, setInclude] = useState("");
  const [busy, setBusy] = useState("");
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState("");
  const [notice, setNotice] = useState("");
  const [noticeTone, setNoticeTone] = useState<"success" | "warning" | "error">("error");
  const [output, setOutput] = useState("");
  const [activity, setActivity] = useState<{ operation: string; time: string; code: number }[]>([]);
  const [dryRun, setDryRun] = useState(true);
  const [readData, setReadData] = useState(false);
  const [policy, setPolicy] = useState({ keepLast: 3, keepDaily: 7, keepWeekly: 4, keepMonthly: 6 });
  const [preview, setPreview] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [stats, setStats] = useState<{ total_size?: number; total_file_count?: number } | null>(null);
  const desktop = isTauri();
  const synchronizedProfiles = useRef(new Map<string, string>());
  const synchronization = useRef<Promise<void>>(Promise.resolve());
  const [scheduleSyncError, setScheduleSyncError] = useState("");
  /** Serialize profile snapshot updates so background backups use the latest saved settings. */
  function syncProfiles(current: Profile[]) {
    synchronization.current = synchronization.current.then(async () => {
      let failure = "";
      for (const item of current) {
        const serialized = JSON.stringify(item);
        if (synchronizedProfiles.current.get(item.id) === serialized) continue;
        try {
          await schedulerAction("sync", item.id, item);
          synchronizedProfiles.current.set(item.id, serialized);
        } catch (error) { failure = String(error); }
      }
      setScheduleSyncError(failure);
    });
    return synchronization.current;
  }
  useEffect(() => {
    if (!desktop) return;
    const timer = window.setTimeout(() => void syncProfiles(profiles), 500);
    return () => window.clearTimeout(timer);
  }, [profiles, desktop]);
  async function flushScheduleProfiles() {
    if (desktop) await syncProfiles(profiles);
  }
  /** Remove the schedule and stop its service before deleting the local profile. */
  async function removeProfile() {
    if (!window.confirm("Remove this saved profile and its schedule? Repository data and password files will remain.")) return;
    setBusy("profile removal"); setNotice("");
    try {
      if (desktop) { await synchronization.current; await schedulerAction("remove", profile.id); }
      const next = profiles.filter(p => p.id !== active);
      save(next); reset(); setActive(next[0].id);
    } catch (error) { setNoticeTone("error"); setNotice(String(error)); }
    finally { setBusy(""); }
  }
  /** Update profile state and persist the supplied profiles in local storage. Storage errors propagate to the caller. */
  function save(next: Profile[]) { setProfiles(next); localStorage.setItem("restic-profiles", JSON.stringify(next)); }
  /** Clear the session password, loaded repository data, connection status, retention preview, output, and notice when switching profiles. Activity history remains session-wide. */
  function reset() { setPassword(""); setSnapshots([]); setSelected(""); setFiles([]); setConnected(false); setPreview(""); setConfirmation(""); setStats(null); setOutput(""); setNotice(""); }
  /** Persist one field of the active profile. Repository or password-file changes invalidate loaded connection data and retention previews. */
  function update<K extends keyof Profile>(key: K, value: Profile[K]) { save(profiles.map(p => p.id === active ? { ...p, [key]: value } : p)); if (key === "repository" || key === "passwordFile") { setConnected(false); setSnapshots([]); setSelected(""); setFiles([]); setStats(null); setPreview(""); } }
  /** Invoke run_restic with active credentials and options. Ignore calls while busy; clear previous output, record up to 20 completed process results, and show a success, warning, or error notice. Return the result on exit code 0, otherwise null. Extra options override the standard request fields. */
  async function run(operation: string, extra: Record<string, unknown> = {}) {
    if (busy) return null;
    setCancelling(false); setCancelError("");
    setBusy(operation); setNotice(""); setNoticeTone("error"); setOutput("");
    try {
      if (!desktop) throw new Error("Open the desktop app with npm run tauri dev to run restic. Browser mode only previews the interface.");
      const result = await invoke<Result>("run_restic", { request: { repository: profile.repository, password, passwordFile: profile.passwordFile, operation, maintenance: profile.maintenance, paths: lines(profile.sources), excludes: lines(profile.excludes), tags: lines(profile.tags), snapshot: selected, target, include, ...policy, confirmation, ...extra } });
      setOutput([result.stdout, result.stderr].filter(Boolean).join("\n"));
      setActivity(a => [{ operation, time: new Date().toLocaleTimeString(), code: result.code }, ...a].slice(0, 20));
      if (result.code === 130) {
        setNoticeTone("warning");
        setNotice(`${operation} cancelled. Changes already made are not undone. Refresh repository data before continuing.`);
        setConnected(false); setSnapshots([]); setSelected(""); setFiles([]); setStats(null); setPreview("");
        return null;
      }
      if (result.code !== 0) {
        setNoticeTone(result.code === 3 ? "warning" : "error");
        throw new Error(result.code === 3 ? "Backup was incomplete: some source files could not be read. Review the output." : `restic ${operation} exited with code ${result.code}. Review the output.`);
      }
      setNoticeTone("success");
      setNotice(`${operation} completed successfully.`);
      return result;
    } catch (error) { setNotice(String(error instanceof Error ? error.message : error)); return null; }
    finally { setBusy(""); setCancelling(false); }
  }
  /** Run snapshots and parse its JSON array. Mark the repository connected only on successful parsing, then clear the snapshot selection, file listing, and retention preview. */
  /** Request cancellation of the current app-owned restic operation. */
  async function cancelOperation() {
    setCancelling(true); setCancelError("");
    try { await invoke("cancel_restic"); }
    catch (error) {
      setCancelling(false);
      setCancelError(error instanceof Error ? error.message : String(error));
    }
  }
  /** Create a managed password file and persist its returned location in the profile. */
  async function createPasswordFile() {
    if (busy) return;
    setBusy("password file creation"); setNotice(""); setNoticeTone("error");
    try {
      if (!desktop) throw new Error("Open the desktop app to create a password file.");
      const path = await invoke<string>("create_password_file", { profileId: profile.id, password });
      update("passwordFile", path);
      setNoticeTone("success");
      setNotice("Password file created. This profile will use it for repository operations.");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    } finally { setBusy(""); setCancelling(false); }
  }
  async function refresh() {
    const result = await run("snapshots");
    if (result) { try { const data = JSON.parse(result.stdout) as Snapshot[]; setSnapshots((data || []).sort((a, b) => Date.parse(b.time) - Date.parse(a.time))); setConnected(true); setSelected(""); setFiles([]); setPreview(""); } catch { setNoticeTone("error"); setNotice("Could not parse restic snapshots."); setConnected(false); } }
    else setConnected(false);
  }
  // Reconnect once at startup; defer so StrictMode's effect cleanup cancels its first attempt.
  useEffect(() => {
    if (!desktop || !profile.repository.trim()) return;
    const timer = window.setTimeout(() => {
      if (profile.passwordFile.trim()) void refresh();
      else {
        setTab("Repository");
        setNoticeTone("warning");
        setNotice("Enter your session password and connect, or save a password file to reconnect automatically next time.");
      }
    }, 0);
    return () => window.clearTimeout(timer);
  }, []);
  /** Load only a snapshot directory's immediate children; keep restore selection independent. */
  async function browse(id: string, directory = "/", revealRestore = false) {
    if (busy) return;
    if (revealRestore) setRestoreRequest(n => n + 1);
    if (id !== selected) setInclude("");
    setSelected(id); setFiles([]); setBrowseDirectory(directory); setFilePage(0);
    const result = await run("ls", { snapshot: id, directory });
    if (result) { try {
      const nodes = result.stdout.split("\n").filter(Boolean).map(line => JSON.parse(line));
      setFiles(nodes.filter(node => node.struct_type === "node" && node.path !== directory)
        .sort((a: Entry, b: Entry) => Number(b.type === "dir") - Number(a.type === "dir") || a.path.localeCompare(b.path)));
    } catch { setNoticeTone("error"); setNotice("Could not parse the snapshot file listing."); } }
  }
  const chosen = snapshots.find(s => s.id === selected);
  const filtered = snapshots.filter(s => `${s.id} ${s.hostname} ${s.paths.join(" ")} ${s.tags?.join(" ")}`.toLowerCase().includes(query.toLowerCase()));
  const policyKey = JSON.stringify([active, profile.repository, policy]);
  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark">◈</span><div>Restic<span>BACKUP MANAGER</span></div></div>
      <div className="nav-label">WORKSPACE</div>
      <nav aria-label="Main navigation">{["Overview", "Repository", "Backup", "Snapshots", "Maintenance"].map((name, i) => <button key={name} className={tab === name ? "nav-item active" : "nav-item"} onClick={() => { setTab(name); setQuery(""); }} aria-current={tab === name ? "page" : undefined}><span aria-hidden="true">{["◫", "▣", "↑", "◷", "⚙"][i]}</span>{name}</button>)}</nav>
      <div className="sidebar-bottom"><span className="dot" /> Local desktop workspace<p>Encrypted backups.<br />Recovery you control.</p><button className="text-button" disabled={!!busy} onClick={() => void run("version")}>Check restic version ↗</button></div>
    </aside>
    <main>
      <header><div className="breadcrumb">Workspace <span>/</span> {tab}</div><div className="header-controls"><label className="theme-control">Theme<select value={theme} onChange={e => setTheme(e.target.value as Theme)}><option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option></select></label><div role="status" className={`connection status-${connected ? "success" : "warning"}`}><span className="dot" aria-hidden="true" />{connected ? "Repository connected" : "Not connected"}</div></div></header>
      <div className="page-heading"><div><div className="eyebrow">YOUR DATA, PROTECTED</div><h1>{tab === "Overview" ? "Backup overview" : tab}</h1><p>{({ Overview: "A clear view of your backups, from source to recovery.", Repository: "Connect to an existing repository or create an encrypted one.", Backup: "Choose what matters. Keep a recoverable copy.", Snapshots: "Explore your history and bring your files back.", Maintenance: "Check integrity and manage your retention policy." } as Record<string, string>)[tab]}</p></div><button disabled={!!busy} className="primary" onClick={() => void refresh()}>↻ Connect / refresh</button></div>
      {!desktop && <div className="banner">Interface preview · Run the Tauri desktop app to connect to restic.</div>}
      <div className="profile-bar"><label>Repository<select disabled={!!busy} value={active} onChange={e => { reset(); setActive(e.target.value); }}>{profiles.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select></label><code>{profile.repository || "No location configured"}</code><button disabled={!!busy} onClick={() => { const p = blank(); save([...profiles, p]); reset(); setActive(p.id); setTab("Repository"); }}>+ Add repository</button></div>
      <div role="status" aria-live="polite" className={`status${busy || notice ? " visible" : ""} status-${busy ? "progress" : noticeTone}`}>{busy ? <><span className="spinner" /> {cancelling ? `Cancelling ${busy}… Waiting for restic to stop.` : `Running ${busy}… Keep the app open until it finishes.`}</> : notice}</div>
      {busy && busy !== "password file creation" && busy !== "profile removal" && <div className="actions cancel-actions"><button className="danger" disabled={cancelling} onClick={() => void cancelOperation()}>{cancelling ? "Cancelling…" : "Cancel operation"}</button>{cancelError && <span role="alert" className="result-error">{cancelError}</span>}</div>}
      {scheduleSyncError && <p role="alert" className="result-error">Schedule synchronization: {scheduleSyncError}</p>}
      <fieldset disabled={!!busy} className="workspace">
      {tab === "Overview" && <>
        <div className="metrics"><article><span>AVAILABLE SNAPSHOTS</span><strong>{connected ? snapshots.length : "—"}</strong><small>{connected ? "In this repository" : "Connect to load snapshots"}</small></article><article><span>LATEST BACKUP</span><strong className="date-value">{snapshots.length ? new Date([...snapshots].sort((a,b) => b.time.localeCompare(a.time))[0].time).toLocaleDateString() : "—"}</strong><small>{snapshots.length ? "Most recent snapshot" : "No snapshot loaded"}</small></article><article><span>RESTORE SIZE</span><strong>{stats ? size(stats.total_size) : "—"}</strong><small>{stats ? `${stats.total_file_count ?? 0} files (all snapshots)` : "Load statistics below"}</small></article></div>
        <div className="columns"><section className="card"><div className="card-heading"><h2>Your backup workflow</h2><span className="badge">RESTIC POWERED</span></div>{[["01", "Connect a repository", "Set your storage location and unlock credentials.", "Repository"], ["02", "Create a snapshot", "Back up files with tags and exclusion patterns.", "Backup"], ["03", "Test your recovery", "Browse snapshots and restore into a new folder.", "Snapshots"]].map(([n,title,description,destination]) => <button className="workflow" key={n} onClick={() => setTab(destination)}><span className="step">{n}</span><div><strong>{title}</strong><p>{description}</p></div><span>→</span></button>)}</section><section className="card"><h2>Repository health</h2><p>Check the repository structure regularly, and run a full data check when you need deeper verification.</p><button onClick={() => void run("check")}>Run integrity check</button><button onClick={async () => { const r = await run("stats"); if (r) { try { setStats(JSON.parse(r.stdout)); } catch { setNoticeTone("error"); setNotice("Could not parse repository statistics."); } } }}>Load statistics</button><div className="tip">Keep your repository password somewhere safe. A backup is useful only if you can restore it.</div></section></div>
      </>}
      {tab === "Repository" && <section className="card form-card"><h2>Repository settings</h2><p>Profile settings save automatically on this device. Typed passwords stay in memory unless you explicitly create a password file.</p>{profile.repository.startsWith("sftp:") && <p className="banner">SFTP uses SSH authentication separately from the repository encryption password. Configure an SSH key or agent and verify the server’s host key outside the app. Connections run without password or host-trust prompts.</p>}<label>Profile name<input value={profile.name} onChange={e => update("name", e.target.value)} /></label><label>Repository location<input placeholder="/mnt/backups/restic or sftp:user@host:/backup" value={profile.repository} onChange={e => update("repository", e.target.value)} spellCheck={false} /></label><div className="form-grid"><label>Session password<input type="password" autoComplete="off" value={password} onChange={e => { setPassword(e.target.value); setPreview(""); }} /></label><label>Password file (takes priority)<input placeholder="Created automatically, or enter an existing file" value={profile.passwordFile} onChange={e => update("passwordFile", e.target.value)} /></label></div><button disabled={!password} onClick={() => void createPasswordFile()}>Create password file</button><p>Create a private password file automatically under ~/.local/share/tauri-backup-manager/passwords. Its path is saved in this profile. Existing files are never overwritten. Supported on Linux and macOS.</p><p>Use an absolute local path or a restic backend URL. Remote storage credentials are inherited from the app’s environment. URLs, paths and profile settings are stored unencrypted; avoid embedding credentials in a repository URL.</p><div className="actions"><button className="primary" onClick={() => void refresh()}>Connect existing repository</button><button onClick={() => { if (window.confirm(`Initialize a new encrypted repository at ${profile.repository}? Keep a safe copy of your password.`)) void run("init"); }}>Initialize new repository</button><button onClick={() => setPassword("")}>Clear session password</button>{profiles.length > 1 && <button className="danger" onClick={() => void removeProfile()}>Remove profile</button>}</div></section>}
      {tab === "Backup" && <section className="card form-card"><h2>Create a backup</h2><label>Source paths · one absolute path per line<textarea rows={5} placeholder={"/home/you/Documents\n/home/you/Pictures"} value={profile.sources} onChange={e => update("sources", e.target.value)} /></label><div className="form-grid"><label>Exclude patterns · one per line<textarea rows={4} placeholder={"*.tmp\n**/node_modules"} value={profile.excludes} onChange={e => update("excludes", e.target.value)} /></label><label>Tags · one per line<textarea rows={4} placeholder={"documents\nmanual"} value={profile.tags} onChange={e => update("tags", e.target.value)} /></label></div><label className="checkbox"><input type="checkbox" checked={dryRun} onChange={e => setDryRun(e.target.checked)} /> Dry run · preview without creating a snapshot</label><div className="actions"><button className="primary" onClick={async () => { const r = await run("backup", { dryRun }); if (r && !dryRun) { setConnected(false); setStats(null); setPreview(""); } }}>{dryRun ? "Preview backup" : "Back up now"} ↑</button></div><p>Sources must exist on this computer. After backing up, refresh snapshots to see the new recovery point.</p></section>}
      {tab === "Snapshots" && <><section className="card"><div className="card-heading"><h2>Snapshot history <span className="count">{snapshots.length}</span></h2><input aria-label="Filter snapshots" placeholder="Filter by host, path, tag or ID…" value={query} onChange={e => setQuery(e.target.value)} /></div>{!filtered.length ? <div className="empty">{connected ? "No matching snapshots. Create a backup to get started." : "Connect to your repository to load its snapshots."}</div> : <div className="table-scroll snapshot-history-list" role="region" aria-label="Snapshot history" tabIndex={0}><table><thead><tr><th>Snapshot</th><th>Created</th><th>Host / sources</th><th>Tags</th><th /></tr></thead><tbody>{filtered.map(s => <tr key={s.id} className={selected === s.id ? "selected" : ""}><td><code>{s.short_id || s.id.slice(0,8)}</code></td><td>{new Date(s.time).toLocaleString()}</td><td>{s.hostname}<small>{s.paths.join(", ")}</small></td><td>{s.tags?.join(", ") || "—"}</td><td><button onClick={() => void browse(s.id, "/", true)}>Browse / restore</button></td></tr>)}</tbody></table></div>}</section>{chosen && <section className="card form-card"><h2 ref={restoreHeading} tabIndex={-1} className="restore-heading">Restore snapshot <code>{chosen.short_id}</code></h2><p>Restores are verified and never overwrite existing files. Choose a new or empty directory.</p><label>Restore destination<input value={target} onChange={e => setTarget(e.target.value)} placeholder="/home/you/Recovered" /></label><label>Include path or pattern (optional)<input value={include} onChange={e => setInclude(e.target.value)} placeholder="Leave blank for the entire snapshot" /></label><button className="primary" onClick={() => { if (window.confirm(`Restore snapshot ${chosen.short_id} into ${target}?`)) void run("restore"); }}>Restore files ↓</button><h3>Snapshot files · {files.length} in this folder</h3><p>Open a folder to browse it. Select a file, or use the current folder as the restore filter.</p><div className="actions"><button disabled={browseDirectory === "/"} onClick={() => void browse(selected, browseDirectory.slice(0, browseDirectory.lastIndexOf("/")) || "/")}>↑ Parent folder</button><button onClick={() => setInclude(browseDirectory === "/" ? "" : browseDirectory)}>Use current folder for restore</button><button onClick={() => void browse(selected, browseDirectory)}>↻ Reload folder</button></div><p>Current folder: <code>{browseDirectory}</code></p><div className="file-list">{files.slice(filePage * 200, (filePage + 1) * 200).map(f => <button key={f.path} onClick={() => f.type === "dir" ? void browse(selected, f.path) : setInclude(f.path)}><span>{f.type === "dir" ? "▣" : "·"} {f.path.slice(f.path.lastIndexOf("/") + 1)}</span><small>{f.type === "dir" ? "Open folder →" : size(f.size)}</small></button>)}</div>{!files.length && <p>{busy === "ls" ? "Loading folder…" : "No entries loaded. Reload this folder to try again."}</p>}{files.length > 200 && <div className="actions"><button disabled={filePage === 0} onClick={() => setFilePage(filePage - 1)}>Previous files</button><span>Page {filePage + 1} of {Math.ceil(files.length / 200)}</span><button disabled={(filePage + 1) * 200 >= files.length} onClick={() => setFilePage(filePage + 1)}>Next files</button></div>}</section>}</>}
      {tab === "Maintenance" && <><section className="card form-card"><h2>Automatic maintenance</h2>
        <label className="checkbox"><input type="checkbox" checked={!!profile.maintenance} onChange={e => update("maintenance", e.target.checked ? { ...policy, readData: false } : undefined)} />After each successful backup: check integrity, apply retention, then prune</label>
        <p>Applies to manual and scheduled backups. Dry runs and unsuccessful backups skip maintenance. A failed step stops the remaining steps. Settings save automatically for this repository.</p>
        {profile.maintenance && <>
          <label className="checkbox"><input type="checkbox" checked={profile.maintenance.readData} onChange={e => update("maintenance", { ...profile.maintenance!, readData: e.target.checked })} />Read all backup data during the integrity check</label>
          <div className="retention-grid">{(["keepLast", "keepDaily", "keepWeekly", "keepMonthly"] as const).map((key, i) => <label key={key}>{["Keep last", "Daily", "Weekly", "Monthly"][i]}<input type="number" min="0" max="10000" value={profile.maintenance![key]} onChange={e => update("maintenance", { ...profile.maintenance!, [key]: Math.max(0, Math.min(10000, Math.trunc(Number(e.target.value)))) })} /></label>)}</div>
          <p className="result-warning">Automatic retention deletes snapshots outside this policy; pruning reclaims their unused storage. Keep at least one retention count above zero.</p>
        </>}
      </section><section className="card form-card"><h2>Verify repository</h2><p>Structural checks validate repository metadata. Reading all data also validates stored pack contents and can take much longer.</p><label className="checkbox"><input type="checkbox" checked={readData} onChange={e => setReadData(e.target.checked)} /> Read and verify all stored data</label><button onClick={() => void run("check", { readData })}>Run integrity check</button></section><section className="card form-card"><h2>Snapshot retention</h2><p>Keep recovery points per host and source-path group. Preview the policy before removing any snapshots.</p><div className="retention-grid">{(Object.keys(policy) as (keyof typeof policy)[]).map((k,i) => <label key={k}>{["Keep last", "Daily", "Weekly", "Monthly"][i]}<input type="number" min="0" max="10000" value={policy[k]} onChange={e => { setPolicy({ ...policy, [k]: Math.max(0, Math.min(10000, Math.trunc(Number(e.target.value)))) }); setPreview(""); }} /></label>)}</div><div className="actions"><button onClick={async () => { const r = await run("forget", { dryRun: true }); setPreview(r ? policyKey : ""); setConfirmation(""); }}>Preview retention</button><label>Type FORGET to confirm<input value={confirmation} onChange={e => setConfirmation(e.target.value)} autoComplete="off" /></label><button className="danger" disabled={preview !== policyKey || confirmation !== "FORGET"} onClick={async () => { const r = await run("forget"); setPreview(""); setConfirmation(""); if (r) { setSnapshots([]); setSelected(""); setFiles([]); setStats(null); setConnected(false); } }}>Apply retention</button></div></section><section className="card form-card"><h2>Reclaim unused storage</h2><p>Prune permanently removes data no longer referenced by snapshots. Run after reviewing and applying retention.</p><button className="danger" onClick={() => { if (window.prompt("Permanently reclaim unused data? Type PRUNE to continue.") === "PRUNE") void run("prune", { confirmation: "PRUNE" }); }}>Prune repository</button></section></>}
      </fieldset>
      {tab === "Backup" && <SchedulePanel key={profile.id} profile={profile} busy={!!busy} beforeSave={flushScheduleProfiles} />}
      <section className="card output-card"><div className="card-heading"><h2>Operation output</h2><span className="badge">{busy ? "RUNNING" : "SESSION LOG"}</span></div><pre tabIndex={0}>{output || "Command output will appear here when an operation finishes."}</pre>{activity.length > 0 && <details><summary>Recent operations ({activity.length})</summary>{activity.map((a,i) => <div className="activity" key={i}><span>{a.time} · {a.operation}</span><span className={`result-${a.code === 0 ? "success" : (a.code === 3 || a.code === 130) ? "warning" : "error"}`}>{a.code === 0 ? "Completed" : a.code === 130 ? "Cancelled" : `Exit ${a.code}`}</span></div>)}</details>}</section>
      <footer>Backup Manager for Restic <span>Local control · Encrypted storage · Verified recovery</span></footer>
    </main>
  </div>;
}
export default App;
