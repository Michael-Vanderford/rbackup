import assert from "node:assert/strict";
import { test } from "node:test";
import { formatOperationOutput } from "../src/formatOperationOutput.ts";

test("formats mixed Restic output and journal entries without losing errors", () => {
    const formatted = formatOperationOutput([
        '{"message_type":"status","percent_done":0.9}',
        '{"message_type":"status","percent_done":0.2}',
        '2026-11-19T04:00:00-06:00 host rbackup[11]: {"message_type":"error","error":{"message":"permission denied"},"item":"/disk/lost+found"}',
        "Backup continued with unreadable files skipped.",
        '{"message_type":"summary","dry_run":true,"files_new":4}',
    ].join("\n"));

    assert.match(formatted, /2 Restic progress updates/);
    assert.match(formatted, /2026-11-19.*rbackup/);
    assert.match(formatted, /Message: permission denied/);
    assert.match(formatted, /Item: \/disk\/lost\+found/);
    assert.match(formatted, /Backup continued with unreadable files skipped/);
    assert.match(formatted, /Backup preview summary/);
    assert.match(formatted, /Files new: 4/);
    assert.ok(formatted.endsWith("2 Restic progress updates · see raw output"));
});

test("formats standalone JSON and preserves unparseable lines", () => {
    assert.match(formatOperationOutput('[{"hostname":"laptop","paths":["/home"]}]'), /Paths\n\s+• \/home/);
    assert.equal(formatOperationOutput("some output {missing JSON}"), "some output {missing JSON}");
    assert.equal(formatOperationOutput("[]"), "None");
});

test("shows the summary first in untimestamped command output", () => {
    const formatted = formatOperationOutput([
        '{"message_type":"status","percent_done":0.2}',
        '{"message_type":"summary","dry_run":true,"files_new":4}',
    ].join("\n"));
    assert.ok(formatted.startsWith("Backup preview summary"));
    assert.ok(formatted.endsWith("1 Restic progress update · see raw output"));
});

test("sorts Restic JSON records by their own time fields", () => {
    const oldest = { time: "2026-11-19T03:00:01.473048794-06:00", id: "oldest" };
    const middle = { time: "2026-11-19T03:00:01.473048795-06:00", id: "middle" };
    const newest = { time: "2026-11-19T02:00:02.001-07:00", id: "newest" };
    const records = [oldest, middle, newest];
    for (const data of [records, [{ keep: records }]]) {
        const formatted = formatOperationOutput(JSON.stringify(data));
        assert.match(formatted, /Id: newest[\s\S]+Id: middle[\s\S]+Id: oldest/);
    }
});

test("sorts timestamped entries newest first and keeps their continuation lines", () => {
    const formatted = formatOperationOutput([
        "Untimestamped header",
        "2026-11-19T05:00:00-05:00 host older entry",
        "Details for older entry",
        '2026-11-19T04:30:00-06:00 host rbackup: {"message_type":"summary"}',
        "2026-11-19T04:30:00-06:00 host same timestamp",
    ].join("\n"));

    assert.match(formatted, /^.*same timestamp\n\n.*rbackup:.*\nSummary\n\n.*older entry\n\nDetails for older entry\n\nUntimestamped header$/);
    assert.equal(formatOperationOutput("First line\nSecond line"), "Second line\n\nFirst line");
});
