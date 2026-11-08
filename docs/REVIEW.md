# Implementation review — 2026-11-07

## Delivered

- Replaced the starter with a responsive desktop workspace: overview, repository setup, backup, snapshots/restore and maintenance.
- Multiple automatically saved repository profiles with session-only typed passwords or password files.
- Real restic operations through a single asynchronous Tauri command: version, init, snapshots, backup, ls, restore, stats, check, forget and prune.
- Backup dry run defaults on; source paths must exist and be absolute. Excludes and tags use separate argument values.
- Snapshot filters by ID, host, path and tag; file browser can populate a restore include path.
- Restore requires a new/empty absolute target, verifies content and uses `--overwrite never`.
- Retention is previewed before UI application; Rust independently requires explicit destructive confirmation text and a nonzero bounded retention policy.
- No shell interpolation, no arbitrary command/flag API, closed stdin, inherited password-command disabled, one running restic job per process. CSP restricts content to bundled assets and Tauri IPC.
- Responsive layout, labeled fields, visible keyboard focus, reduced-motion support, status announcements and no URL hashes for view navigation.

## Validation

- `npm run build`: passed (TypeScript and Vite production build).
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: passed validation test; real lifecycle test is intentionally opt-in.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings`: passed.
- Real-restic lifecycle: passed against restic 0.18.1 (13.08 seconds), including a byte-for-byte restore comparison, actual retention removal, pruning, password-file authentication and incorrect-password handling. The initial sandboxed attempt reached restore but failed on restricted ownership restoration (`lchown`) and cache creation; rerunning the generated-data test outside the filesystem sandbox passed.
- Visual/UI smoke test: blocked because the computer/browser automation host was unavailable. The development server starts successfully. Desktop behavior, dialogs, focus and packaged rendering still need manual review.

## Review priorities

1. Run `npm run tauri dev` and complete a backup/recovery trial using disposable data on the target machine.
2. Add native file pickers and keychain storage before asking less technical users to configure paths and credentials.
3. Add streaming output, cancellation with child-process cleanup and window-close handling before lengthy production jobs. Current command output is buffered and jobs cannot be cancelled in the UI.
4. Stream/paginate file and snapshot listings; current loading and output capture are unbounded in memory, although file rendering stops at 1,000 entries.
5. Add OS scheduling with persistent results and notifications. A desktop timer is insufficient for reliable unattended backup.
6. Test remote backends and macOS/Windows packaging. The local test uses restic 0.18.1 on Linux.
7. Consider backend-issued retention-preview tokens if additional command callers or background jobs are introduced. Preview gating currently belongs to the UI; deletion confirmation and policy validation also exist in Rust.

## Deliberate scope limits

No automatic unlock, repair, insecure transport bypass, custom shell command field, or arbitrary restic flag editor. Advanced copy/diff/mount/key-management workflows remain in the plan. No changes were made to real user repositories.

## Job import follow-up

`src/imported-profiles.json` contains a sanitized projection of `docs/jobs.json`. It is merged into desktop local storage once, keyed by the original job IDs. Existing profiles win ID conflicts, and later removals remain removed. Passwords are deliberately omitted from this generated asset; the original JSON is untouched. No repository was opened, initialized, or backed up during import.

Validation: `node tests/profiles.test.ts` passed, covering field mapping, all 39 exclusions, duplicate prevention, existing-profile preservation, and edits/removals across reloads. `npm run build` passed. Job schedules and historical backup dates are not imported into live status.
