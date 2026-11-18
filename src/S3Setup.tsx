import { useState } from "react";
import { s3Repository } from "./profiles";

export default function S3Setup({ useLocation }: { useLocation: (location: string) => void }) {
  const [endpoint, setEndpoint] = useState("");
  const [bucket, setBucket] = useState("");
  const [prefix, setPrefix] = useState("");
  const location = s3Repository(endpoint, bucket, prefix);

  return <details>
    <summary>Set up an S3 repository</summary>
    <p>Use the service endpoint for AWS S3 or an S3-compatible provider. Configure AWS-compatible credentials in the environment used by rbackup and any scheduled jobs.</p>
    <form onSubmit={event => {
      event.preventDefault();
      if (location) useLocation(location);
    }}>
      <div className="form-grid">
        <label>S3 endpoint<input required value={endpoint} onChange={event => setEndpoint(event.target.value)} placeholder="https://s3.us-east-1.amazonaws.com" spellCheck={false} /></label>
        <label>Bucket<input required value={bucket} onChange={event => setBucket(event.target.value)} placeholder="my-backups" spellCheck={false} /></label>
        <label>Repository prefix (optional)<input value={prefix} onChange={event => setPrefix(event.target.value)} placeholder="rbackup/desktop" spellCheck={false} /></label>
      </div>
      {location && <p><code>{location}</code></p>}
      {endpoint.trim() && bucket.trim() && !location && <p className="result-error">Enter an HTTP or HTTPS service endpoint, a valid bucket name, and a prefix without dot segments. Put the bucket and prefix in their own fields.</p>}
      <button type="submit" disabled={!location}>Use S3 location</button>
    </form>
  </details>;
}
