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
- Guided repository connection setup (scope below).
- Large repository pagination/streaming for snapshot file listings.
- Packaging/signing and cross-platform desktop smoke tests.
- Advanced restic operations: copy, diff, mount, key management, repair and unlock. These need their own workflows, particularly repair/unlock.

Do not describe this first implementation as complete coverage of every restic subcommand or production-ready without the remaining platform testing.

## Guided repository connections

The [Restic 0.19.1 repository guide](https://restic.readthedocs.io/en/v0.19.1/030_preparing_a_new_repo.html) documents nine backend families. rbackup already accepts repository locations directly and passes them to Restic. The shared setup should add guided fields, configuration guidance, and verification for every family while preserving the custom location field and existing saved URLs.

| Backend | Guided fields and configuration | Current coverage |
| --- | --- | --- |
| Local | Absolute directory path, including Windows drive paths | Existing path input |
| SFTP | SSH host or alias, optional user and port, and relative or absolute remote path. Use external SSH identity or agent and verified host keys. | URL input and non-interactive SSH handling |
| REST server | HTTP(S) endpoint and optional repository path. Support external REST credentials and a custom CA reference if needed. Document Unix sockets as an advanced location format. | URL input |
| Amazon S3 and S3-compatible storage | Endpoint, bucket, optional prefix and region. Account for path-style versus virtual-hosted bucket lookup where applicable. Use external AWS-compatible credentials. Include examples for Amazon S3, MinIO, Wasabi, Alibaba OSS, and Backblaze B2 via S3. | Guided URL form with external credentials and region settings |
| OpenStack Swift | Container and optional path. Use the external Swift or OpenStack authentication environment. | URL input |
| Backblaze B2 native | Bucket and optional prefix. Use external B2 application-key credentials. Point new setups to S3-compatible access as Restic recommends. | URL input |
| Azure Blob Storage | Storage account, container, path and optional endpoint suffix. Use external Azure authentication. The account and endpoint settings are separate from the repository URL. | URL input |
| Google Cloud Storage | Bucket and optional path, with a project ID or external credentials-file reference where needed. Google Drive uses rclone. | URL input |
| rclone | Configured remote name and repository path. Use the installed rclone executable and configuration visible to the app. | Guided URL form and disposable local remote verification |

### 1. Build the shared setup

- [ ] Add a connection-type selector and guided fields for the backend families above. Preview generated locations and preserve existing URLs as entered. Start with local, rclone, SFTP, REST and S3, then cover Azure Blob, Google Cloud Storage, Swift and native B2.
- [ ] Keep repository encryption credentials in the existing session-password or password-file flow. Use external provider authentication and allowlisted per-profile settings or file references where required. Pass the same settings to connection checks, initialization, backup, restore, mounting where supported, maintenance and scheduled jobs.
- [ ] Keep custom locations available for advanced syntax. Do not add provider sign-in, provisioning, bundled binaries, or arbitrary executable command strings to this scope.

### 2. Diagnose connection problems

- [ ] Run bounded checks off the UI thread. For a standard named rclone remote, distinguish a missing executable, unknown remote or unreadable configuration. Use Restic to test repository access for all backends.
- [ ] Surface authentication, TLS or SSH trust, network, repository password, and missing repository errors when the tools distinguish them. Preserve diagnostic output with credentials redacted. Offer **Initialize new repository** only when Restic reports a missing repository, with explicit confirmation.

### 3. Verify manual and scheduled operation

- [ ] Cover generated locations, custom URL preservation, and backend option forwarding with focused tests. For each backend family, record whether connection, explicit initialization, backup, snapshot listing and verified restore passed with a disposable endpoint or account.
- [ ] Verify Linux systemd user timers and Windows Task Scheduler under the scheduled account. Confirm a new snapshot and, with maintenance enabled on a test repository, confirm prune in the scheduled output. Exercise an rclone remote and an externally authenticated backend on both schedulers. Confirm that missing credentials or configuration produce actionable errors.
- [ ] Document setup and scheduler requirements per backend. Record any provider or platform verification still pending before describing it as tested.

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
