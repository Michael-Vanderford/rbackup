# Restic desktop interface — implementation plan

## Goal
Turn the Tauri starter into a usable desktop interface for the full local backup lifecycle, with real restic commands rather than simulated data.

## Implementation
- [x] Repository profiles, connection testing, initialization, session-only passwords or password files.
- [x] Backup source paths, excludes, tags and dry runs.
- [x] Snapshot listing, filtering, file browsing and verified restore into an empty destination.
- [x] Repository statistics, integrity checks, retention preview/apply and pruning.
- [x] Validated Rust command boundary, asynchronous execution, activity output and explicit destructive confirmations.
- [x] Real restic lifecycle tests, frontend build and Rust checks.

## Decisions
Use the installed restic executable and existing React/Tauri dependencies. Pass arguments directly without a shell. Store profile metadata locally; never persist a typed password. Remote backend credentials come from the environment of the desktop process. Keep retention grouped by restic's default host and paths. Require a fresh preview before applying a policy in the UI.

## Follow-up review
- OS keychain integration and native folder/file pickers.
- Notification delivery and non-Linux scheduling. Linux user timers and missed-run catch-up are implemented.
- Streaming progress, cancellation and durable activity history.
- Credential forms for remote storage providers; test S3, SFTP, REST and rclone on each platform.
- Large repository pagination/streaming for snapshot file listings.
- Packaging/signing and cross-platform desktop smoke tests.
- Advanced restic operations: copy, diff, mount, key management, repair and unlock. These need their own workflows, particularly repair/unlock.

Do not describe this first implementation as complete coverage of every restic subcommand or production-ready without the remaining platform testing.

## Clean first launch

- Personal job imports and their path migration have been removed.
- Fresh installs start with a blank repository profile; existing saved profiles are preserved.
- Regression checks cover empty storage, persisted profiles, and invalid saved data.

## Theme support

- [x] Add a header theme selector with System, Light and Dark choices.
- [x] Remember the selected preference locally; follow OS theme changes in System mode.
- [x] Style forms, cards, tables, output, status messages and destructive actions for dark mode, preserving focus indicators and responsive layout.

## Linux scheduler

- [x] Daily/weekly schedules, disabled by default, with next-run/status/journal display.
- [x] Private schedule metadata and password-file-only unattended authentication.
- [x] Headless backup entry point and persistent systemd user timers.
- [x] Cross-process coordination, queued work, and cancellation through service shutdown.
- [x] Profile edit synchronization, invalid-setting disable, and removal cleanup.
- [x] Isolated headless backup/cancellation tests, unit generation checks, and opt-in runtime timer test.

See README's scheduler section for runtime requirements, verification commands, and limits. Maintenance remains manual; no system-wide service, automatic lingering changes, cron editor, or notifications are included.
