use super::*;

impl SqliteSessionStorage {
    pub(super) fn prune(&self, cutoff: Timestamp) -> StorageResult<()> {
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            let ids=self.with_conn(StorageErrorKind::Query,|c|c.prepare("SELECT id FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ? ORDER BY id LIMIT 50")?.query_map([timestamp(cutoff)?],|r|r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>())?;
            if ids.is_empty() {
                break;
            }
            for id in ids {
                for table in ["events", "messages", "approvals", "attachments"] {
                    if !self.delete_chunks(table, Some(id), until, true)? {
                        return Ok(());
                    }
                }
                if Instant::now() >= until {
                    return Ok(());
                }
                self.with_conn(StorageErrorKind::Write, |c| {
                    c.execute("DELETE FROM sessions WHERE id=?", [id])
                        .map(|_| ())
                })?;
            }
        }
        let _ = self.with_conn(StorageErrorKind::Write, |c| {
            c.execute_batch("PRAGMA incremental_vacuum(20000); PRAGMA wal_checkpoint(TRUNCATE);")
        });
        Ok(())
    }
    pub(super) fn prune_noise(&self) -> StorageResult<i64> {
        let mut deleted = 0;
        let mut last = 0;
        loop {
            let batch=self.with_conn(StorageErrorKind::Query,|c|c.prepare("SELECT m.id,COALESCE(m.raw_text,m.text,'') FROM messages m JOIN sessions se ON se.id=m.session_id WHERE m.id>? AND m.role='ai' AND se.provider IN ('claude','codex') ORDER BY m.id LIMIT 2000")?.query_map([last],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>())?;
            if batch.is_empty() {
                break;
            }
            last = batch.last().unwrap().0;
            let ids = batch
                .into_iter()
                .filter(|(_, body)| text::noise(&text::visible(body)))
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            if ids.is_empty() {
                continue;
            }
            deleted += self.with_conn(StorageErrorKind::Write, |c| {
                let tx = c.transaction()?;
                let placeholders = vec!["?"; ids.len()].join(",");
                if self.inner.fts {
                    tx.execute(
                        &format!("DELETE FROM messages_fts WHERE rowid IN ({placeholders})"),
                        params_from_iter(ids.iter()),
                    )?;
                }
                let count = tx.execute(
                    &format!("DELETE FROM messages WHERE id IN ({placeholders})"),
                    params_from_iter(ids.iter()),
                )?;
                tx.commit()?;
                Ok(count as i64)
            })?;
        }
        Ok(deleted)
    }
    /// Each chunk releases the connection; reset holds only the history barrier.
    /// The boolean reports budget completion separately from a query failure.
    fn delete_chunks(
        &self,
        table: &str,
        session: Option<i64>,
        until: Instant,
        delete_fts: bool,
    ) -> StorageResult<bool> {
        debug_assert!(["events", "messages", "approvals", "attachments"].contains(&table));
        loop {
            if Instant::now() >= until {
                return Ok(false);
            }
            let count = self.with_conn(StorageErrorKind::Write, |c| {
                let select = match session {
                    Some(_) => format!("SELECT id FROM {table} WHERE session_id=? LIMIT 2000"),
                    None => format!("SELECT id FROM {table} LIMIT 2000"),
                };
                let ids = c
                    .prepare(&select)?
                    .query_map(params_from_iter(session), |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if ids.is_empty() {
                    return Ok(0);
                }
                let placeholders = vec!["?"; ids.len()].join(",");
                let tx = c.transaction()?;
                if table == "messages" && self.inner.fts && delete_fts {
                    tx.execute(
                        &format!("DELETE FROM messages_fts WHERE rowid IN ({placeholders})"),
                        params_from_iter(ids.iter()),
                    )?;
                }
                tx.execute(
                    &format!("DELETE FROM {table} WHERE id IN ({placeholders})"),
                    params_from_iter(ids.iter()),
                )?;
                tx.commit()?;
                Ok(ids.len())
            })?;
            if count < 2000 {
                return Ok(true);
            }
            thread::yield_now();
        }
    }
    pub(super) fn reset(&self, preserve: &[LiveSessionId]) -> StorageResult<ResetResult> {
        let _barrier = self
            .inner
            .history
            .write()
            .unwrap_or_else(|e| e.into_inner());
        self.reset_under_barrier(preserve)
    }
    // Only called with history's write lock held. Kept separate so deterministic
    // queue fixtures can exercise the exact production reset body under a held barrier.
    pub(super) fn reset_under_barrier(
        &self,
        preserve: &[LiveSessionId],
    ) -> StorageResult<ResetResult> {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        let result = self.reset_sql(preserve);
        match result {
            Ok(out) => {
                // Full VACUUM has a longer maintenance budget than ordinary queries.
                let vacuum = self.with_conn(StorageErrorKind::Reset, |c| {
                    schema::deadline(c, Instant::now() + Duration::from_secs(60))?;
                    c.execute_batch("VACUUM")?;
                    let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
                    Ok(())
                });
                if vacuum.is_err() {
                    self.schedule_file_reset()?;
                }
                Ok(out)
            }
            Err(err) => {
                let _ = self.schedule_file_reset();
                Err(err)
            }
        }
    }
    fn reset_sql(&self, preserve: &[LiveSessionId]) -> StorageResult<ResetResult> {
        let until = Instant::now() + Duration::from_secs(60);
        let (mut out, preserved) = self.with_conn(StorageErrorKind::Reset, |c| {
            let count = |table| {
                c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
            };
            let out = ResetResult {
                sessions: count("sessions")?,
                events: count("events")?,
                messages: count("messages")?,
                approvals: count("approvals")?,
                attachments: count("attachments")?,
                preserved: 0,
            };
            let mut ids = Vec::new();
            for live in preserve.iter().filter(|id| id.0 > 0) {
                if let Some(id) = resolve(c, *live, false)?
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
            Ok((out, ids))
        })?;
        out.preserved = preserved.len() as i64;
        if self.inner.fts {
            self.with_conn(StorageErrorKind::Reset, |c| {
                c.execute("DELETE FROM messages_fts", []).map(|_| ())
            })?;
        }
        for table in ["events", "messages", "approvals", "attachments"] {
            // FTS was cleared as one independent statement. Deleting those same
            // external-content entries twice can corrupt its index.
            if !self.delete_chunks(table, None, until, false)? {
                return Err(error(
                    StorageErrorKind::Timeout,
                    "history reset maintenance budget expired",
                ));
            }
        }
        self.with_conn(StorageErrorKind::Reset,|c|{
            if preserved.is_empty(){c.execute("DELETE FROM sessions",[])?;}else{
                let placeholders=vec!["?";preserved.len()].join(",");
                c.execute(&format!("DELETE FROM sessions WHERE id NOT IN ({placeholders})"),params_from_iter(preserved.iter()))?;
                let mut values=vec![rusqlite::types::Value::from(now())];values.extend(preserved.into_iter().map(rusqlite::types::Value::from));
                c.execute(&format!("UPDATE sessions SET first_message=NULL,last_message=NULL,title=NULL,tags_json=NULL,summary=NULL,updated_at=? WHERE id IN ({placeholders})"),params_from_iter(values))?;
            }
            Ok(out)
        })
    }
}
