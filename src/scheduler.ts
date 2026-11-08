import { invoke } from "@tauri-apps/api/core";
import type { Profile } from "./profiles";

export type Schedule = { enabled: boolean; frequency: "daily" | "weekly"; time: string; weekdays: number[] };
export type ScheduledRun = { state: "waiting" | "running" | "success" | "incomplete" | "cancelled" | "failed"; started: number; finished: number | null; code: number | null; message: string };
export type ScheduleStatus = { available: boolean; reason: string; schedule: Schedule; active: boolean; serviceError: string; nextRun: string; lastRun: ScheduledRun | null; logs: string };
export const defaultSchedule = (): Schedule => ({ enabled: false, frequency: "daily", time: "03:00", weekdays: [0] });

/** Invoke the typed scheduler boundary; profiles contain metadata and password-file paths only. */
export function schedulerAction<T = void>(action: "status" | "save" | "sync" | "remove" | "cancel", profileId: string, profile?: Profile, schedule?: Schedule): Promise<T> {
    return invoke<T>("scheduler_action", { action, profileId, profile: profile ?? null, schedule: schedule ?? null });
}
