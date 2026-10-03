use super::*;

pub(super) fn run(inner: Arc<Inner>, receiver: mpsc::Receiver<QueuedHistoryEvent>) {
    for queued in receiver {
        if inner.stop.load(Ordering::Acquire) {
            inner.pending.fetch_sub(1, Ordering::AcqRel);
            inner.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let barrier = inner.history.read().unwrap_or_else(|e| e.into_inner());
        let result = if queued.generation.0 != inner.generation.load(Ordering::Acquire) {
            inner.discarded.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            let result = inner.with_conn(StorageErrorKind::Write, |c| {
                repository::store_event(c, inner.fts, queued.live_session_id, &queued.event)
            });
            if result.is_ok() {
                inner.written.fetch_add(1, Ordering::Relaxed);
            }
            result
        };
        drop(barrier);
        inner.pending.fetch_sub(1, Ordering::AcqRel);
        // Invoke application observers without database/history locks held.
        inner.notify(queued.live_session_id, result);
    }
    inner.done.store(true, Ordering::Release);
}
impl SqliteSessionStorage {
    pub(super) fn begin_close(&self) {
        // Same short lock as enqueue: no new accepted event can arrive after take.
        let mut sender = lock(&self.inner.sender);
        self.inner.closing.store(true, Ordering::Release);
        sender.take();
    }
    pub(super) fn ensure_external_close(&self) -> StorageResult<()> {
        // Never acquire a lock that a foreground closer holds while joining
        // this writer. The immutable ID remains available after JoinHandle is
        // taken, so observer reentry cannot wait on close_lock or writer.
        if self.writer_id == thread::current().id() {
            return Err(error(
                StorageErrorKind::Closed,
                "writer observer cannot close its own repository",
            ));
        }
        Ok(())
    }
    pub(super) fn finish_close(&self) -> StorageResult<()> {
        self.ensure_external_close()?;
        let _serial = lock(&self.close_lock);
        let mut handle = lock(&self.writer);
        if let Some(worker) = handle.take() {
            worker.join().map_err(|_| {
                error(
                    StorageErrorKind::Write,
                    "database writer stopped unexpectedly",
                )
            })?;
        }
        if let Some(connection) = lock(&self.inner.connection).take() {
            connection
                .close()
                .map_err(|(_, e)| sql_error(StorageErrorKind::Write, e))?;
        }
        Ok(())
    }
    pub(super) fn shutdown_report(&self) -> ShutdownReport {
        ShutdownReport {
            written: self.inner.written.load(Ordering::Acquire),
            discarded_old_generation: self.inner.discarded.load(Ordering::Acquire),
            dropped: self.inner.dropped.load(Ordering::Acquire).max(0) as u64,
            remaining: self.inner.pending.load(Ordering::Acquire),
            error: lock(&self.inner.last_error).clone(),
            ..ShutdownReport::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The outer case supervises a real child test process so a regression in
    /// callback/close lock ordering cannot hang the entire test runner.
    #[test]
    fn observer_close_during_concurrent_close_is_bounded() {
        const CHILD: &str = "MANY_AI_STORAGE_CLOSE_OBSERVER_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let root = tempfile::tempdir().unwrap();
            let installed = tempfile::tempdir().unwrap();
            let paths = RuntimePaths::trial(root.path(), 49115, installed.path()).unwrap();
            let store = Arc::new(
                SqliteSessionStorage::open(
                    &paths,
                    StorageOptions::baseline(paths.resource(Resource::Logs)),
                )
                .unwrap(),
            );
            store
                .start_session(SessionStart {
                    live_session_id: LiveSessionId(1),
                    ..Default::default()
                })
                .unwrap();
            store.inner.with_conn(StorageErrorKind::Write,|c|c.execute_batch("CREATE TRIGGER reject_async BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'synthetic'); END")).unwrap();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let (observer_tx, observer_rx) = mpsc::channel();
            let release = Mutex::new(release_rx);
            let weak = Arc::downgrade(&store);
            store.set_on_write_error(Some(Arc::new(move |_, _| {
                entered_tx.send(()).unwrap();
                lock(&release).recv_timeout(Duration::from_secs(2)).unwrap();
                let result = weak.upgrade().unwrap().close();
                observer_tx.send(result).unwrap();
            })));
            store.store_event_async(
                LiveSessionId(1),
                HistoryEvent(
                    serde_json::json!({"type":"reject"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            );
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let (close_tx, close_rx) = mpsc::channel();
            let closing = store.clone();
            let closer = thread::spawn(move || {
                close_tx.send(closing.close()).unwrap();
            });
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                if matches!(
                    store.close_lock.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ) {
                    break;
                }
                assert!(
                    Instant::now() < until,
                    "foreground close did not acquire its lock"
                );
                thread::sleep(Duration::from_millis(1));
            }
            release_tx.send(()).unwrap();
            assert_eq!(
                observer_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("writer observer deadlocked against close")
                    .unwrap_err()
                    .kind,
                StorageErrorKind::Closed
            );
            close_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("foreground close did not finish")
                .unwrap();
            closer.join().unwrap();
            store.set_on_write_error(None);
            store.close().unwrap();
            return;
        }
        use std::process::{Command, Stdio};
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::writer::tests::observer_close_during_concurrent_close_is_bounded",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(6);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= until {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("close observer regression exceeded subprocess deadline");
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "close observer subprocess failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
