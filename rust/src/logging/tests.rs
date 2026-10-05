use super::*;
use std::io::Read;
#[test]
fn rotation_preserves_bytes_gzip_retention_and_unrelated_files() {
    let root = tempfile::tempdir().unwrap();
    let directory = Arc::new(Dir::open_or_create_private(root.path()).unwrap());
    let log = RollingLog::new(directory.clone(), "auto-approval.jsonl").unwrap();
    let config = LogConfig {
        max_size_mb: 1,
        max_backups: 1,
        compress: true,
        ..Default::default()
    };
    directory
        .create_new("unrelated.txt", b"untouched", 0o600)
        .unwrap();
    let first = vec![b'a'; 1024 * 1024 - 1];
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-05T01:02:03.004Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    log.write_at(&config, &first, now).unwrap();
    log.write_at(&config, b"bc", now).unwrap();
    let backup = "auto-approval-2026-10-05T01-02-03.004.jsonl.gz";
    let mut restored = Vec::new();
    flate2::read::GzDecoder::new(directory.open_file(backup, false).unwrap())
        .read_to_end(&mut restored)
        .unwrap();
    assert_eq!(restored, first);
    assert_eq!(directory.read("auto-approval.jsonl", 10).unwrap(), b"bc");
    log.write_at(&config, &first, now + chrono::Duration::seconds(1))
        .unwrap();
    assert!(!directory.path().join(backup).exists());
    assert_eq!(directory.read("unrelated.txt", 20).unwrap(), b"untouched");
    assert_eq!(log.backups().unwrap().len(), 1);
}
#[test]
fn oversized_write_is_rejected_before_creation_and_close_reopens_at_inclusive_limit() {
    let root = tempfile::tempdir().unwrap();
    let directory = Arc::new(Dir::open_or_create_private(root.path()).unwrap());
    let log = RollingLog::new(directory.clone(), "hub.log").unwrap();
    let config = LogConfig {
        max_size_mb: 1,
        ..Default::default()
    };
    assert!(log.write(&config, &vec![0; 1024 * 1024 + 1]).is_err());
    assert!(!directory.path().join("hub.log").exists());
    log.write(&config, &vec![b'x'; 1024 * 1024 - 1]).unwrap();
    log.close().unwrap();
    log.write(&config, b"y").unwrap();
    assert_eq!(directory.read("hub.log", 10).unwrap(), b"y");
    assert_eq!(log.backups().unwrap().len(), 1);
}
#[cfg(unix)]
#[test]
fn log_symlink_cannot_escape_private_directory() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("hub.log")).unwrap();
    let directory = Arc::new(Dir::open_or_create_private(root.path()).unwrap());
    let log = RollingLog::new(directory, "hub.log").unwrap();
    assert!(log.write(&LogConfig::default(), b"private").is_err());
    assert_eq!(std::fs::metadata(outside.path()).unwrap().len(), 0);
}
