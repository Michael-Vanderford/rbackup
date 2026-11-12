import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { toggleSelection, within } from "./treeSelection";

export type Entry = { path: string; type: string; size?: number; mtime?: string | number };
export type Snapshot = { id: string; short_id: string; time: string; hostname: string; paths: string[]; tags?: string[] };
const name = (path: string) => path.split("/").filter(Boolean).pop() || "Snapshot root";
const size = (n = 0) => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`;

function SelectionBox({ checked, partial, label, disabled, toggle }: { checked: boolean; partial: boolean; label: string; disabled: boolean; toggle: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { if (input.current) input.current.indeterminate = partial; }, [partial]);
  return <input ref={input} type="checkbox" tabIndex={-1} checked={checked} aria-label={label} disabled={disabled} onChange={toggle} />;
}

export default function SnapshotBrowser({ snapshot, loadDirectory, restore, busy, mount, mountBusy, mountedPath, mountError, unmount, cancel, cancelling, close }: {
  snapshot: Snapshot;
  loadDirectory: (snapshot: string, directory: string) => Promise<Entry[]>;
  restore: (paths: string[], destination: string) => Promise<boolean>;
  busy: boolean;
  mount: () => void;
  mountBusy: boolean;
  mountedPath?: string;
  mountError: string;
  unmount?: () => void;
  cancel: () => void;
  cancelling: boolean;
  close: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const tree = useRef<HTMLUListElement>(null);
  const alive = useRef(true);
  const requests = useRef(new Map<string, Promise<Entry[]>>());
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const load = useRef(loadDirectory);
  load.current = loadDirectory;
  const [children, setChildren] = useState(new Map<string, Entry[]>());
  const [expanded, setExpanded] = useState(new Set(["/"]));
  const [loading, setLoading] = useState(new Set<string>());
  const [errors, setErrors] = useState(new Map<string, string>());
  const [shown, setShown] = useState(new Map<string, number>());
  const [selection, setSelection] = useState(new Set<string>());
  const [target, setTarget] = useState("");
  const [error, setError] = useState("");
  const [restored, setRestored] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const locked = busy || mountBusy || restoring;

  function ensure(path: string) {
    if (requests.current.has(path)) return;
    setLoading(previous => new Set(previous).add(path));
    setErrors(previous => { const next = new Map(previous); next.delete(path); return next; });
    // Restic operations are serialized; rapidly expanding folders queues their listings.
    const request = queue.current.catch(() => {}).then(() => {
      if (!alive.current) throw new Error("Browser closed.");
      return load.current(snapshot.id, path);
    });
    requests.current.set(path, request);
    queue.current = request;
    request.then(entries => {
      if (alive.current) setChildren(previous => new Map(previous).set(path, entries));
    }).catch(error => {
      requests.current.delete(path);
      if (alive.current) setErrors(previous => new Map(previous).set(path, String(error)));
    }).finally(() => {
      if (alive.current) setLoading(previous => { const next = new Set(previous); next.delete(path); return next; });
    });
  }
  useEffect(() => {
    alive.current = true;
    const element = dialog.current!;
    element.showModal();
    ensure("/");
    return () => { alive.current = false; element.close(); };
  }, []);
  function expand(path: string) {
    setExpanded(previous => {
      const next = new Set(previous);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
    if (!expanded.has(path)) ensure(path);
  }
  function toggle(path: string) {
    try { setSelection(toggleSelection(selection, path, children)); setRestored(false); setError(""); }
    catch (error) { setError(String(error)); }
  }
  function keyboard(event: KeyboardEvent<HTMLLIElement>, entry: Entry) {
    if (locked || (event.target as HTMLElement).closest("[data-tree-path]") !== event.currentTarget) return;
    const rows = Array.from(tree.current?.querySelectorAll<HTMLElement>("[data-tree-path]") || []);
    const current = event.currentTarget;
    const index = rows.indexOf(current);
    let next: HTMLElement | undefined;
    if (event.key === "ArrowDown") next = rows[index + 1];
    else if (event.key === "ArrowUp") next = rows[index - 1];
    else if (event.key === "Home") next = rows[0];
    else if (event.key === "End") next = rows[rows.length - 1];
    else if (event.key === "ArrowRight" && entry.type === "dir") {
      if (!expanded.has(entry.path)) expand(entry.path);
      else if (rows[index + 1] && within(rows[index + 1].dataset.treePath!, entry.path)) next = rows[index + 1];
    } else if (event.key === "ArrowLeft") {
      if (entry.type === "dir" && expanded.has(entry.path)) expand(entry.path);
      else next = rows.find(row => row.dataset.treePath === (entry.path.slice(0, entry.path.lastIndexOf("/")) || "/"));
    } else if (event.target === current && event.key === " ") toggle(entry.path);
    else if (event.target === current && event.key === "Enter" && entry.type === "dir") expand(entry.path);
    else return;
    event.preventDefault(); next?.focus();
  }
  async function doRestore() {
    if (!selection.size || !target.trim() || locked || loading.size) return;
    setRestoring(true); setRestored(false); setError("");
    try { const done = await restore([...selection], target.trim()); setRestored(done); if (!done) setError("Restore did not complete. Review the operation output."); }
    catch (error) { setError(String(error)); }
    finally { setRestoring(false); }
  }
  function renderNode(entry: Entry, depth: number) {
    const checked = [...selection].some(parent => within(entry.path, parent));
    const partial = !checked && [...selection].some(path => within(path, entry.path));
    const open = expanded.has(entry.path);
    const entries = children.get(entry.path);
    const limit = shown.get(entry.path) || 100;
    return <li key={entry.path} role="treeitem" aria-level={depth + 1}
      aria-expanded={entry.type === "dir" ? open : undefined} aria-checked={partial ? "mixed" : checked}
      data-tree-path={entry.path} tabIndex={0} onKeyDown={event => keyboard(event, entry)}>
      <div className={`tree-row${checked ? " selected" : ""}`} style={{ paddingLeft: 12 + depth * 20 }}>
        {entry.type === "dir" ? <button className="tree-toggle" tabIndex={-1} disabled={locked} onClick={() => expand(entry.path)} aria-label={`${open ? "Collapse" : "Expand"} ${name(entry.path)}`} aria-expanded={open}>{open ? "▾" : "▸"}</button> : <span className="tree-toggle" />}
        <SelectionBox checked={checked} partial={partial} label={`Select ${entry.path}`} disabled={locked} toggle={() => toggle(entry.path)} />
        <button className="tree-name" title={entry.path} tabIndex={-1} disabled={locked} onClick={() => entry.type === "dir" ? expand(entry.path) : toggle(entry.path)}><span className="tree-icon" aria-hidden="true">{entry.type === "dir" ? <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round"><path d="M3 7V5a1 1 0 0 1 1-1h5l2 3h9a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Z" /><path d="M3 8h18" /></svg> : entry.type === "symlink" ? "↗" : "·"}</span>{name(entry.path)}</button>
        <span className="tree-size">{entry.type === "dir" ? "Folder" : size(entry.size)}</span>
        <span className="tree-modified">{entry.mtime ? new Date(entry.mtime).toLocaleString() : ""}</span>
      </div>
      {entry.type === "dir" && open && <ul role="group">
        {loading.has(entry.path) && <li role="none" className="tree-message" style={{ paddingLeft: 48 + depth * 20 }}><span className="spinner" />Loading…</li>}
        {errors.has(entry.path) && <li role="none" className="tree-message result-error"><span role="alert">{errors.get(entry.path)}</span><button disabled={locked} onClick={() => ensure(entry.path)}>Retry</button></li>}
        {entries?.slice(0, limit).map(child => renderNode(child, depth + 1))}
        {entries?.length === 0 && <li role="none" className="tree-message" style={{ paddingLeft: 48 + depth * 20 }}>Empty folder</li>}
        {entries && entries.length > limit && <li role="none" className="tree-message"><button onClick={() => setShown(previous => new Map(previous).set(entry.path, limit + 100))}>Show next {Math.min(100, entries.length - limit)} items ({entries.length - limit} remaining)</button></li>}
      </ul>}
    </li>;
  }
  return <dialog ref={dialog} className="snapshot-browser" aria-labelledby="browser-title" onCancel={event => { event.preventDefault(); if (!locked) close(); }}>
    <div className="browser-heading"><div><h2 id="browser-title">{mountedPath ? "Mounted snapshot" : "Snapshot browser"} <code>{snapshot.short_id || snapshot.id.slice(0, 8)}</code></h2><p>{snapshot.hostname}{snapshot.time ? ` · ${new Date(snapshot.time).toLocaleString()}` : ""}</p></div><div className="actions">{!mountedPath && <button disabled={locked || !!loading.size} onClick={mount}>{mountBusy ? "Mounting…" : "Mount snapshot"}</button>}{unmount && <button disabled={locked || !!loading.size} onClick={unmount}>Unmount</button>}<button disabled={locked} onClick={close}>Close ✕</button></div></div>
    {mountedPath && <div className="mounted-path"><label>Mounted snapshot path<input readOnly value={mountedPath} onFocus={event => event.target.select()} /></label><button onClick={() => { void (navigator.clipboard?.writeText(mountedPath) || Promise.reject()).catch(() => setError("Select the mounted path above and copy it with Ctrl+C.")); }}>Copy path</button></div>}
    <div className="browser-toolbar"><p>Expand folders and check the files or folders to recover. Selecting a folder includes its contents.</p><button disabled={locked} onClick={() => setExpanded(new Set())}>Collapse all</button><button disabled={locked || !selection.size} onClick={() => { setSelection(new Set()); setRestored(false); }}>Clear selection</button></div>
    {mountError && <p role="alert" className="result-error">{mountError}</p>}{error && <p role="alert" className="result-error">{error}</p>}
    <div className="browser-files tree-files"><ul ref={tree} role="tree" aria-label={mountedPath ? "Mounted snapshot files" : "Snapshot files"} aria-multiselectable="true">{renderNode({ path: "/", type: "dir" }, 0)}</ul></div>
    <section className="browser-restore" aria-label="Restore selection"><div><strong>{selection.has("/") ? "Entire snapshot selected" : `${selection.size} file${selection.size === 1 ? "" : "s"} / folder${selection.size === 1 ? "" : "s"} selected`}</strong><p>Original paths are preserved in a new or empty destination. Restored files are verified.</p><div className="tree-selection-summary">{[...selection].map(path => <code key={path}>{path}</code>)}</div></div><label>Restore destination<input value={target} disabled={locked} onChange={event => { setTarget(event.target.value); setRestored(false); }} placeholder="/home/you/Recovered" /></label><button className="primary" disabled={locked || !!loading.size || !selection.size || !target.trim()} onClick={() => void doRestore()}>{restoring ? "Restoring…" : "Restore selected"}</button>{restoring && <button disabled={cancelling} onClick={cancel}>{cancelling ? "Cancelling…" : "Cancel restore"}</button>}{restored && <p role="status" className="result-success">Restore completed and verified.</p>}</section>
  </dialog>;
}
