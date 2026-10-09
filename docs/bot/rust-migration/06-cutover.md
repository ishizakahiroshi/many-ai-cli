# #3 many-ai-cli rust: C6 copied data, operator acceptance and cutover

> 最終更新: 2026-10-03(土) 12:07:41

## Prerequisites

Use only the reviewed candidate SHA and immutable artifacts from C5. Save both old Go binaries and the matching new binaries/assets. Preserve config/profiles/DB plus WAL/SHM consistently, runtime metadata and hashes as one recoverable set. Do not put data content or credentials in GitHub progress.

## Copied-data and rollback rehearsal

1. Begin with synthetic databases, then explicitly selected copied data. Quiesce writers before backup or use a consistency-preserving backup method; copying a live DB file without WAL consistency is not proof.
2. Verify old schema/columns, FTS enabled and fallback, busy locks, generation reset, prune, history/session/approval/attachment identity, YAML/JSON/JSONL formats and private permissions. Keep old/new writers separated.
3. Start the candidate using the trial root. Exercise representative read/write/history/settings/profile flows on the copy. Stop only trial-owned processes, restore the original copied state and verify the old Go version can read/write it.
4. If a format becomes irreversible, stop before applying it to real data, document the change and obtain that specific decision. Successful Rust reads do not prove rollback.

## Operator acceptance matrix

Observe actual Windows and Unix PTY/input/resize/close/child termination and headless timeout. Check approval answers and replay under reconnect; all providers' transcript/native behavior; both launcher binaries/profile UI/SSH serve and tunnel; update through safe stubs before real installer acceptance; all Web screens including mobile; native voice/runtime and Windows tray/stop/autostart. WSL must not be launched on the operator's Windows machine by an AI.

Check effective remote/proxy Host/Origin/PIN/transport behavior and native model/runtime hashes in the actual chosen environments. Synthetic mocks cannot accept these boundaries. Real credential/profile, external notification or quota-consuming provider acceptance must be specifically selected by the operator; retain unperformed cases as pending.

## Serial cutover and stability

With the operator present and all preconditions passed: stop old writer/Hub; verify the recovery set; start the accepted candidate on the intended runtime; perform the agreed representative operations; record observations. Keep old artifacts and recovery data intact.

On failure: stop the new writer; restore the old artifacts and consistent data/config; verify old startup and representative operations. Do not leave both versions writing the same DB. Never use a blanket process-name kill that can terminate other tasks.

The operator identifies the daily workflows and stability observation window before final acceptance. Record start/end, actual operations, error/recovery observations and any pending cases; elapsed time alone is not stability proof. Only after operator acceptance and stable operation may v1.0.0 release be prepared. Use the repository's release process and verify source/tag/CI/artifact/consumer versions separately. Merge, publication and cleanup must match the accepted candidate.

## Done criteria

All K/A rows have accepted evidence; data restoration was rehearsed; operator reports actual cutover and representative use; release channel metadata/artifacts align if published. Close instruction progress accurately and retain/remove only owned worktrees after integration, validation and delivery. Do not mark overall done while manual/OS/data/rollback gates are pending.
