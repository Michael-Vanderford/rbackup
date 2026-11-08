import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import type { Profile } from "./profiles";
import { defaultSchedule, schedulerAction, type ScheduleStatus } from "./scheduler";

/** Edit one repository's schedule and poll systemd without overwriting unsaved edits. */
export default function SchedulePanel({ profile, busy, beforeSave }: { profile: Profile; busy: boolean; beforeSave: () => Promise<void> }) {
    const [schedule, setSchedule] = useState(defaultSchedule);
    const [status, setStatus] = useState<ScheduleStatus | null>(null);
    const [loaded, setLoaded] = useState(false);
    const [saving, setSaving] = useState(false);
    const [cancelling, setCancelling] = useState(false);
    const [message, setMessage] = useState("");
    const [error, setError] = useState("");
    const desktop = isTauri();

    useEffect(() => {
        if (!desktop) return;
        let disposed = false;
        let polling = false;
        let initialized = false;
        async function refresh() {
            if (polling) return;
            polling = true;
            try {
                const next = await schedulerAction<ScheduleStatus>("status", profile.id);
                if (!disposed) {
                    setStatus(next);
                    if (!initialized) { setSchedule(next.schedule); setLoaded(true); initialized = true; }
                    // An invalid profile edit can disable the saved schedule in the backend.
                    // Preserve unsaved form changes; the saved enabled state is displayed separately.
                }
            } catch (error) {
                if (!disposed) setError(String(error));
            } finally { polling = false; }
        }
        void refresh();
        const timer = window.setInterval(() => void refresh(), 5000);
        return () => { disposed = true; window.clearInterval(timer); };
    }, [desktop, profile.id]);

    async function save() {
        setSaving(true); setError(""); setMessage("");
        try {
            await beforeSave();
            await schedulerAction("save", profile.id, profile, schedule);
            setStatus(await schedulerAction<ScheduleStatus>("status", profile.id));
            setMessage(schedule.enabled ? "Schedule enabled. Backups run while you are logged in, even with the app closed." : "Schedule disabled.");
        } catch (error) { setError(String(error)); }
        finally { setSaving(false); }
    }
    async function cancel() {
        setCancelling(true); setError(""); setMessage("");
        try {
            await schedulerAction("cancel", profile.id);
            setStatus(await schedulerAction<ScheduleStatus>("status", profile.id));
            setMessage("Scheduled backup stopped. Completed changes are not undone.");
        } catch (error) { setError(String(error)); }
        finally { setCancelling(false); }
    }
    const run = status?.lastRun;
    const tone = run?.state === "success" ? "success" : run?.state === "failed" ? "error" : run?.state === "running" || run?.state === "waiting" ? "progress" : "warning";
    return <section className="card form-card schedule-card" aria-labelledby="schedule-heading">
        <div className="card-heading"><h2 id="schedule-heading">Backup schedule</h2><span className={`badge schedule-status status-${!status ? "progress" : !status.available ? "warning" : status.schedule.enabled ? "success" : "warning"}`} role="status">{!status ? "LOADING" : !status.available ? "UNAVAILABLE" : status.schedule.enabled ? "ENABLED" : "DISABLED"}</span></div>
        <p>Daily or weekly backups in your computer’s local time. Missed runs catch up once when the user scheduler resumes. Scheduled backups create snapshots, independently of the manual dry-run checkbox.</p>
        {!desktop && <p className="banner">Open the Linux desktop app to configure systemd user timers.</p>}
        {status && !status.available && <div className="banner">Scheduling requires Linux and a working systemd user session. Manual backups remain available.<p>{status.reason}</p></div>}
        <fieldset className="workspace" disabled={!loaded || !status?.available || busy || saving || cancelling}>
            <label className="checkbox"><input type="checkbox" checked={schedule.enabled} onChange={e => setSchedule({ ...schedule, enabled: e.target.checked })} />Enable scheduled backups</label>
            <div className="form-grid"><label>Frequency<select disabled={!schedule.enabled} value={schedule.frequency} onChange={e => setSchedule({ ...schedule, frequency: e.target.value as "daily" | "weekly" })}><option value="daily">Daily</option><option value="weekly">Weekly</option></select></label><label>Local time<input type="time" disabled={!schedule.enabled} required value={schedule.time} onChange={e => setSchedule({ ...schedule, time: e.target.value })} /></label></div>
            {schedule.frequency === "weekly" && <fieldset className="weekday-picker" disabled={!schedule.enabled}><legend>Days of the week</legend>{["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].map((day, index) => <label className="checkbox" key={day}><input type="checkbox" checked={schedule.weekdays.includes(index)} onChange={e => setSchedule({ ...schedule, weekdays: e.target.checked ? [...schedule.weekdays, index] : schedule.weekdays.filter(d => d !== index) })} />{day}</label>)}</fieldset>}
            <p>A saved password file is required. Changes to this profile update its scheduled backup settings; invalid settings disable the schedule.</p>
            <button className="primary" onClick={() => void save()}>{saving ? "Saving…" : "Save schedule"}</button>
        </fieldset>
        <dl className="schedule-details"><div><dt>Next run</dt><dd>{status?.nextRun || "Not scheduled"}</dd></div><div><dt>Current state</dt><dd className={status?.active ? "result-progress" : undefined}>{status?.active ? run?.state === "waiting" ? "Waiting for another operation" : "Running / stopping" : "Idle"}</dd></div><div><dt>Last result</dt><dd>{run ? <span className={`result-${tone}`}>{run.state} · {new Date((run.finished ?? run.started) * 1000).toLocaleString()}{run.code !== null ? ` · Exit ${run.code}` : ""}</span> : "No scheduled backup yet"}</dd></div></dl>
        {run && <p>{run.message}</p>}
        {status?.serviceError && <p role="alert" className="result-error">{status.serviceError}</p>}
        {status?.active && <button className="danger" disabled={cancelling || saving} onClick={() => void cancel()}>{cancelling ? "Cancelling…" : "Cancel scheduled backup"}</button>}
        {message && <p role="status" className="result-success">{message}</p>}
        {error && <p role="alert" className="result-error">{error}</p>}
        <details><summary>Recent scheduled-run output</summary><pre tabIndex={0}>{status?.logs || "Scheduled backup output will appear here."}</pre></details>
    </section>;
}
