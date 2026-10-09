use super::*;

struct WriterCompletion<'a>(&'a Inner);
impl Drop for WriterCompletion<'_> {
    fn drop(&mut self) {
        // Shutdown must join and surface a panicked writer, rather than waiting
        // until its drain deadline for a thread that has already stopped.
        self.0.done.store(true, Ordering::Release);
    }
}

pub(super) fn run(inner: Arc<Inner>, receiver: mpsc::Receiver<QueuedHistoryEvent>) {
    let _completion = WriterCompletion(&inner);
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
        let writer_result = match handle.take() {
            Some(worker) => worker.join().map_err(|_| {
                error(
                    StorageErrorKind::Write,
                    "database writer stopped unexpectedly",
                )
            }),
            None => Ok(()),
        };
        // A failed join is a terminal writer result, not permission to leave
        // the physical connection open. Finish cleanup before returning it.
        let close_result = match lock(&self.inner.connection).take() {
            Some(connection) => connection
                .close()
                .map_err(|(_, e)| sql_error(StorageErrorKind::Write, e)),
            None => Ok(()),
        };
        let result = writer_result.and(close_result);
        if let Err(error) = &result {
            *lock(&self.inner.last_error) = Some(error.clone());
        }
        result
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
    #[tokio::test]
    async fn recovery_core_panicked_observer_reports_stopped_writer_without_timeout_and_closes_connection()
     {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49116, installed.path()).unwrap();
        let store = SqliteSessionStorage::open(
            &paths,
            StorageOptions::baseline(paths.resource(Resource::Logs)),
        )
        .unwrap();
        store
            .start_session(SessionStart {
                live_session_id: LiveSessionId(1),
                ..Default::default()
            })
            .unwrap();
        store.inner.with_conn(StorageErrorKind::Write, |connection| {
            connection.execute_batch("CREATE TRIGGER reject_async BEFORE INSERT ON events WHEN NEW.type='reject' BEGIN SELECT RAISE(ABORT,'synthetic'); END")
        }).unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release = Mutex::new(release_rx);
        store.set_on_write_error(Some(Arc::new(move |_, _| {
            entered_tx.send(()).unwrap();
            lock(&release).recv_timeout(Duration::from_secs(2)).unwrap();
            panic!("synthetic write-error observer panic");
        })));
        let event = |kind: &str| {
            HistoryEvent(
                serde_json::json!({"type":kind,"text":"synthetic"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
        };
        assert!(matches!(
            store.store_event_async(LiveSessionId(1), event("reject")),
            EnqueueOutcome::Queued { .. }
        ));
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..3 {
            assert!(matches!(
                store.store_event_async(LiveSessionId(1), event("user_input")),
                EnqueueOutcome::Queued { .. }
            ));
        }
        release_tx.send(()).unwrap();
        let report = store
            .shutdown(
                ShutdownPolicy::Drain {
                    timeout: Duration::from_secs(1),
                },
                &HubShutdownCancellation::default(),
            )
            .await;
        assert!(
            !report.timed_out,
            "a stopped writer must report failure rather than waiting for a drain timeout: {report:?}"
        );
        assert!(!report.cancelled);
        assert_eq!(report.written, 0);
        assert_eq!(
            report.remaining, 3,
            "accepted work after the failed observer was not persisted"
        );
        assert_eq!(report.error.as_ref().unwrap().kind, StorageErrorKind::Write);
        assert_eq!(
            report.error.as_ref().unwrap().detail,
            "database writer stopped unexpectedly"
        );
        assert!(
            lock(&store.inner.connection).is_none(),
            "failed writer join must not skip closing the physical connection"
        );
        let repeated = store
            .shutdown(
                ShutdownPolicy::Drain {
                    timeout: Duration::from_secs(1),
                },
                &HubShutdownCancellation::default(),
            )
            .await;
        assert!(!repeated.timed_out);
        assert_eq!(repeated.error, report.error);
        store.set_on_write_error(None);
    }

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
