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
    pub(super) fn finish_close(&self) -> StorageResult<()> {
        let _serial = lock(&self.close_lock);
        let mut handle = lock(&self.writer);
        if handle
            .as_ref()
            .is_some_and(|worker| worker.thread().id() == thread::current().id())
        {
            return Err(error(
                StorageErrorKind::Closed,
                "writer observer cannot close its own repository",
            ));
        }
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
