import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import type { Profile } from "./profiles";
import { defaultSchedule, schedulerAction, type Schedule, type ScheduleStatus } from "./scheduler";

/** Autosave one repository’s schedule and poll the native scheduler. */
export default function SchedulePanel({ profile, busy, beforeSave }: { profile: Profile; busy: boolean; beforeSave: () => Promise<void> }) {
    const [schedule, setSchedule] = useState(defaultSchedule);
    const [status, setStatus] = useState<ScheduleStatus | null>(null);
    const [loaded, setLoaded] = useState(false);
    const [saving, setSaving] = useState(false);
    const [timeDraft, setTimeDraft] = useState<string | null>(null);
    const saves = useRef(Promise.resolve());
    const pending = useRef(0);
    const revision = useRef(0);
    const [cancelling, setCancelling] = useState(false);
    const [error, setError] = useState("");
    const desktop = isTauri();

    useEffect(() => {
        if (!desktop) return;
        let disposed = false;
        let polling = false;
        async function refresh() {
            if (polling || pending.current) return;
            polling = true;
            const current = revision.current;
            try {
                const next = await schedulerAction<ScheduleStatus>("status", profile.id);
                if (!disposed && current === revision.current) {
                    setStatus(next);
                    setSchedule(next.schedule);
                    setLoaded(true);
                }
            } catch (error) {
                if (!disposed && current === revision.current) setError(String(error));
            } finally { polling = false; }
        }
        void refresh();
        const timer = window.setInterval(() => void refresh(), 5000);
        return () => { disposed = true; window.clearInterval(timer); };
    }, [desktop, profile.id]);

    function save(next: Schedule) {
        setSchedule(next);
        setSaving(true); setError("");
        pending.current++;
        const current = ++revision.current;
        saves.current = saves.current.then(async () => {
            try {
                await beforeSave();
                await schedulerAction("save", profile.id, profile, next);
            } catch (error) {
                if (current === revision.current) setError(String(error));
            } finally {
                pending.current--;
            }
            if (current !== revision.current) return;
            try {
                const saved = await schedulerAction<ScheduleStatus>("status", profile.id);
                if (current === revision.current) {
                    setStatus(saved);
                    setSchedule(saved.schedule);
                }
            } catch (error) {
                if (current === revision.current) setError(String(error));
            } finally {
                if (current === revision.current) setSaving(false);
            }
        });
    }
    async function cancel() {
        setCancelling(true); setError("");
        try {
            await schedulerAction("cancel", profile.id);
            setStatus(await schedulerAction<ScheduleStatus>("status", profile.id));
        } catch (error) { setError(String(error)); }
        finally { setCancelling(false); }
    }
    const run = status?.lastRun;
    const tone = run?.state === "success" ? "success" : run?.state === "failed" ? "error" : run?.state === "running" || run?.state === "waiting" ? "progress" : "warning";
    return <section className="card form-card schedule-card" aria-labelledby="schedule-heading">
        <div className="card-heading"><h2 id="schedule-heading">Backup schedule</h2><span className={`badge schedule-status status-${!status ? "progress" : !status.available ? "warning" : status.schedule.enabled ? "success" : "warning"}`} role="status">{!status ? "LOADING" : !status.available ? "UNAVAILABLE" : status.schedule.enabled ? "ENABLED" : "DISABLED"}</span></div>
        <p>This schedule applies to <strong>{profile.name}</strong>. Daily and weekly backups use your computer’s local time. The system scheduler can catch up on missed runs. Scheduled backups create snapshots, regardless of the manual dry-run setting.</p>
        {!desktop && <p className="banner">Open the Windows or Linux desktop app to configure schedules.</p>}
        {status && !status.available && <div className="banner">Scheduling requires Windows Task Scheduler or a working Linux systemd user session. Manual backups remain available.<p>{status.reason}</p></div>}
        <fieldset className="workspace" disabled={!loaded || !status?.available || busy || cancelling}>
            <label className="checkbox"><input type="checkbox" role="switch" checked={schedule.enabled} disabled={!schedule.enabled && schedule.frequency === "weekly" && !schedule.weekdays.length} onChange={e => save({ ...schedule, enabled: e.target.checked })} />Enable scheduled backups</label>
            <div className="form-grid"><label>Frequency<select value={schedule.frequency} onChange={e => save({ ...schedule, frequency: e.target.value as Schedule["frequency"], weekdays: schedule.weekdays.length ? schedule.weekdays : [0] })}><option value="daily">Daily</option><option value="weekly">Weekly</option></select></label><label>Local time<input type="time" required value={timeDraft ?? schedule.time}
                onChange={e => setTimeDraft(e.target.value)}
                onBlur={e => {
                    setTimeDraft(null);
                    if (e.target.validity.valid && e.target.value !== schedule.time) save({ ...schedule, time: e.target.value });
                }}
                onKeyDown={e => { if (e.key === "Enter") e.currentTarget.blur(); }} /></label></div>
            {schedule.frequency === "weekly" && <fieldset className="weekday-picker"><legend>Days of the week</legend>{["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].map((day, index) => <label className="checkbox" key={day}><input type="checkbox" checked={schedule.weekdays.includes(index)} disabled={schedule.weekdays.length === 1 && schedule.weekdays.includes(index)} onChange={e => save({ ...schedule, weekdays: e.target.checked ? [...schedule.weekdays, index] : schedule.weekdays.filter(d => d !== index) })} />{day}</label>)}</fieldset>}
            <p>Changes save automatically. Scheduled backups require a saved password file.</p>
        </fieldset>
        <dl className="schedule-details"><div><dt>Next run</dt><dd>{saving ? "Updating…" : status?.schedule.enabled === false ? "Disabled" : status?.nextRun || "Not scheduled"}</dd></div><div><dt>Current state</dt><dd className={status?.active ? "result-progress" : undefined}>{status?.active ? run?.state === "waiting" ? "Waiting for another operation" : "Running / stopping" : "Idle"}</dd></div><div><dt>Last result</dt><dd>{run ? <span className={`result-${tone}`}>{run.state} · {new Date((run.finished ?? run.started) * 1000).toLocaleString()}{run.code !== null ? ` · Exit ${run.code}` : ""}</span> : "No scheduled backup yet"}</dd></div></dl>
        {run && <p>{run.message}</p>}
        {status?.serviceError && <p role="alert" className="result-error">{status.serviceError}</p>}
        {status?.active && <button className="danger" disabled={cancelling || saving} onClick={() => void cancel()}>{cancelling ? "Cancelling…" : "Cancel scheduled backup"}</button>}
        {error && <p role="alert" className="result-error">{error}</p>}
        <details><summary>Recent scheduled-run output</summary><pre tabIndex={0}>{status?.logs || "Scheduled backup output will appear here."}</pre></details>
    </section>;
}
