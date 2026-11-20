function label(key: string) {
    const words = key.replace(/_/g, " ");
    return words.charAt(0).toUpperCase() + words.slice(1);
}

function orderDatedRecords(records: unknown[]): unknown[] {
    const dated = [];
    for (const record of records) {
        if (record === null || typeof record !== "object" || !("time" in record) || typeof record.time !== "string") return records;
        const time = Date.parse(record.time);
        if (!Number.isFinite(time)) return records;
        // Date.parse truncates Restic's fractional seconds to milliseconds.
        const fraction = record.time.match(/T\d{2}:\d{2}:\d{2}\.(\d{4,})/)?.[1];
        const submillisecond = fraction?.padEnd(9, "0").slice(3, 9) ?? "000000";
        dated.push({ record, time, submillisecond });
    }
    return dated.sort((a, b) => b.time - a.time || b.submillisecond.localeCompare(a.submillisecond)).map(entry => entry.record);
}

function fields(value: unknown, indent = ""): string {
    if (Array.isArray(value)) {
        if (!value.length) return "None";
        return orderDatedRecords(value).map(item => `${indent}• ${fields(item, indent + "  ").trimStart()}`).join("\n");
    }
    if (value !== null && typeof value === "object") {
        return Object.entries(value).map(([key, item]) => {
            const nested = item !== null && typeof item === "object" && (!Array.isArray(item) || item.length > 0);
            return nested
                ? `${indent}${label(key)}\n${fields(item, indent + "  ")}`
                : `${indent}${label(key)}: ${fields(item)}`;
        }).join("\n");
    }
    return value === null ? "—" : String(value);
}

function parse(text: string): unknown {
    try { return JSON.parse(text); }
    catch { return undefined; }
}

function newestFirst(lines: string[]): string[] {
    const entries: { time: number; lines: string[] }[] = [];
    const leading: string[] = [];
    let current = leading;
    for (const line of lines) {
        // journalctl short-iso puts the timestamp at the start of each entry.
        const timestamp = line.match(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})(?=\s)/)?.[0];
        const time = timestamp ? Date.parse(timestamp) : NaN;
        if (Number.isFinite(time)) {
            current = [];
            entries.push({ time, lines: current });
        }
        current.push(line);
    }
    if (!entries.length) return lines.reverse();
    // The journal arrives oldest first, including entries with the same timestamp.
    return entries.reverse().sort((a, b) => b.time - a.time).flatMap(entry => entry.lines).concat(leading);
}

export function formatOperationOutput(raw: string): string {
    const document = parse(raw);
    if (document !== null && typeof document === "object" && !("message_type" in document)) {
        return fields(document);
    }

    const blocks: string[] = [];
    let progressCount = 0;
    function flushProgress() {
        if (progressCount) blocks.push(`${progressCount} Restic progress update${progressCount === 1 ? "" : "s"} · see raw output`);
        progressCount = 0;
    }
    for (const line of newestFirst(raw.split(/\r?\n/))) {
        // Linux systemd journal entries prepend a timestamp and process name.
        const start = line.indexOf("{");
        const record = start < 0 ? undefined : parse(line.slice(start));
        if (record !== null && typeof record === "object" && "message_type" in record && typeof record.message_type === "string") {
            const { message_type: type, ...details } = record;
            if (type === "status") {
                progressCount++;
                continue;
            }
            flushProgress();
            const heading = type === "summary" && "dry_run" in details && details.dry_run === true ? "Backup preview summary" : label(type);
            blocks.push([line.slice(0, start).trimEnd(), heading, fields(details)].filter(Boolean).join("\n"));
        } else {
            flushProgress();
            if (line.trim()) blocks.push(line);
        }
    }
    flushProgress();
    return blocks.join("\n\n");
}
