import { useState } from "react";
import { rcloneRepository } from "./profiles";

export default function RcloneSetup({ useLocation }: { useLocation: (location: string) => void }) {
  const [remote, setRemote] = useState("");
  const [path, setPath] = useState("");
  const location = rcloneRepository(remote, path);

  return <details>
    <summary>Set up an rclone repository</summary>
    <p>Install rclone on PATH and configure a named remote with <code>rclone config</code>. Run <code>rclone listremotes</code> to find its name. Scheduled backups need access to the same rclone executable and configuration.</p>
    <form onSubmit={event => {
      event.preventDefault();
      if (location) useLocation(location);
    }}>
      <div className="form-grid">
        <label>rclone remote name<input required value={remote} onChange={event => setRemote(event.target.value)} placeholder="cloud" spellCheck={false} /></label>
        <label>Repository path<input value={path} onChange={event => setPath(event.target.value)} placeholder="bucket/backups" spellCheck={false} /></label>
      </div>
      {location && <p><code>{location}</code></p>}
      {remote.trim() && !location && <p className="result-error">Enter a named remote using letters, numbers, spaces, dots, underscores, or hyphens. Start with a letter, number, or underscore.</p>}
      <button type="submit" disabled={!location}>Use rclone location</button>
    </form>
  </details>;
}
