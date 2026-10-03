use super::*;

impl SessionStorage for SqliteSessionStorage {
    fn open(paths: &RuntimePaths, options: StorageOptions) -> StorageResult<Self> {
        if options.queue_capacity == 0
            || options.query_timeout.is_zero()
            || options.init_timeout.is_zero()
        {
            return Err(error(
                StorageErrorKind::InvalidData,
                "storage capacity and deadlines must be positive",
            ));
        }
        let paths = paths.clone().with_log_dir(&options.log_dir).map_err(|_| {
            error(
                StorageErrorKind::Open,
                "invalid explicit storage log directory",
            )
        })?;
        let path = paths.resource(Resource::Database);
        if paths.is_trial() {
            for candidate in [
                &path,
                &companion(&path, "-wal"),
                &companion(&path, "-shm"),
                &companion(&path, ".reset-pending"),
            ] {
                let relative = candidate
                    .strip_prefix(paths.root())
                    .map_err(|_| error(StorageErrorKind::Open, "database is outside trial root"))?;
                paths
                    .checked_child(Resource::Locks, relative)
                    .map_err(|_| {
                        error(StorageErrorKind::Open, "database path escapes trial root")
                    })?;
            }
        }
        let parent = path
            .parent()
            .ok_or_else(|| error(StorageErrorKind::Open, "database parent is missing"))?;
        let directory = Arc::new(directory::open(parent)?);
        schema::apply_pending_reset(&directory)?;
        // All application-owned writes use the held directory capability.
        match directory.create_new(directory::DATABASE, &[], 0o600) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return Err(error(
                    StorageErrorKind::Open,
                    "database file could not be created",
                ));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Change only the opened inode, and release this setup handle before
            // SQLite acquires POSIX locks on its own database descriptor.
            let file = directory
                .open_file(directory::DATABASE, true)
                .map_err(|_| {
                    error(
                        StorageErrorKind::Open,
                        "database file could not be held safely",
                    )
                })?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| {
                    error(
                        StorageErrorKind::Open,
                        "database permissions could not be secured",
                    )
                })?;
        }
        let (connection, fts) =
            schema::open_connection(&path, options.init_timeout, paths.is_trial())?;
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let inner = Arc::new(Inner {
            connection: Mutex::new(Some(connection)),
            path,
            directory,
            fts,
            query_timeout: options.query_timeout,
            history: RwLock::new(()),
            generation: AtomicU64::new(0),
            sender: Mutex::new(Some(sender)),
            closing: AtomicBool::new(false),
            done: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            pending: AtomicU64::new(0),
            written: AtomicU64::new(0),
            discarded: AtomicU64::new(0),
            dropped: AtomicI64::new(0),
            handler: RwLock::new(None),
            last_error: Mutex::new(None),
        });
        let worker_inner = inner.clone();
        let writer = thread::Builder::new()
            .name("session-history-writer".into())
            .spawn(move || writer::run(worker_inner, receiver))
            .map_err(|_| error(StorageErrorKind::Open, "database writer could not start"))?;
        let writer_id = writer.thread().id();
        Ok(Self {
            inner,
            writer_id,
            writer: Mutex::new(Some(writer)),
            close_lock: Mutex::new(()),
        })
    }
    fn close(&self) -> StorageResult<()> {
        self.ensure_external_close()?;
        self.begin_close();
        // Explicitly stronger than the Go six-second best-effort Close: consume
        // every accepted event before closing the only physical connection.
        self.finish_close()
    }
    fn schedule_file_reset(&self) -> StorageResult<()> {
        self.inner
            .directory
            .replace(
                directory::RESET_MARKER,
                format!("{}\n", now()).as_bytes(),
                0o600,
            )
            .map_err(|_| {
                error(
                    StorageErrorKind::Reset,
                    "database reset marker could not be saved",
                )
            })
    }
    fn file_reset_pending(&self) -> bool {
        self.inner
            .directory
            .metadata(directory::RESET_MARKER)
            .is_ok()
    }
    fn set_on_write_error(&self, handler: Option<WriteErrorHandler>) {
        *self
            .inner
            .handler
            .write()
            .unwrap_or_else(|e| e.into_inner()) = handler;
    }
    fn store_event_async(&self, session: LiveSessionId, event: HistoryEvent) -> EnqueueOutcome {
        let sender = lock(&self.inner.sender);
        let Some(sender) = sender.as_ref() else {
            return EnqueueOutcome::Closed;
        };
        let generation = self.history_generation();
        self.inner.pending.fetch_add(1, Ordering::AcqRel);
        match sender.try_send(QueuedHistoryEvent {
            live_session_id: session,
            generation,
            event,
        }) {
            Ok(()) => EnqueueOutcome::Queued { generation },
            Err(mpsc::TrySendError::Full(_)) => {
                self.inner.pending.fetch_sub(1, Ordering::AcqRel);
                EnqueueOutcome::Dropped {
                    cumulative_count: self.inner.dropped.fetch_add(1, Ordering::AcqRel) + 1,
                }
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.inner.pending.fetch_sub(1, Ordering::AcqRel);
                EnqueueOutcome::Unavailable
            }
        }
    }
    fn start_session(&self, mut st: SessionStart) -> StorageResult<DbSessionId> {
        if st.jsonl_path.is_empty() {
            st.jsonl_path = format!("virtual-live-{}", st.live_session_id.0);
        }
        let state = if st.state.trim().is_empty() {
            "standby"
        } else {
            st.state.trim()
        };
        self.with_conn(StorageErrorKind::Write, |c| c.query_row(
            "INSERT INTO sessions (live_session_id,provider,display_name,cwd,branch,label,model,route,shell,state,started_at,log_path,jsonl_path,parent_session_id,role,auto,depth,orchestration_id,board_path,worktree_branch,subscription_id,updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(jsonl_path) DO UPDATE SET
             live_session_id=excluded.live_session_id,provider=excluded.provider,display_name=excluded.display_name,cwd=excluded.cwd,branch=excluded.branch,model=excluded.model,route=excluded.route,shell=excluded.shell,state=excluded.state,started_at=excluded.started_at,log_path=excluded.log_path,parent_session_id=excluded.parent_session_id,role=excluded.role,auto=excluded.auto,depth=excluded.depth,orchestration_id=excluded.orchestration_id,board_path=excluded.board_path,worktree_branch=excluded.worktree_branch,subscription_id=excluded.subscription_id,updated_at=excluded.updated_at,ended_at=NULL,end_reason=NULL RETURNING id",
            params![st.live_session_id.0,st.provider,st.display,st.cwd,st.branch,st.label,st.model,st.route,st.shell,state,st.started_at,st.log_path,st.jsonl_path,st.parent_session_id.0,st.role,st.auto,st.depth,st.orchestration_id.0,st.board_path,st.worktree_branch,st.subscription_id,now()], |r| Ok(DbSessionId(r.get(0)?))))
    }
    fn close_stale_sessions(&self, ended_at: SystemTime, reason: &str) -> StorageResult<i64> {
        self.with_conn(StorageErrorKind::Write, |c| c.execute("UPDATE sessions SET state='disconnected',end_reason=COALESCE(NULLIF(?, ''),end_reason),ended_at=?,updated_at=? WHERE ended_at IS NULL", params![reason,timestamp(ended_at)?,now()]).map(|n| n as i64))
    }
    fn update_session_messages(&self, session: LiveSessionId, first: &str, last: &str) {
        self.execute(session,"UPDATE sessions SET first_message=CASE WHEN first_message='' OR first_message IS NULL THEN ? ELSE first_message END,last_message=?,updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![first,last,now(),session.0]);
    }
    fn update_session_state(&self, session: LiveSessionId, state: &str, last_output_at: &str) {
        if state.trim().is_empty() && last_output_at.trim().is_empty() {
            return;
        }
        self.execute(session,"UPDATE sessions SET state=COALESCE(NULLIF(?, ''),state),last_output_at=COALESCE(NULLIF(?, ''),last_output_at),updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![state,last_output_at,now(),session.0]);
    }
    fn session_card_meta_by_live_session(
        &self,
        session: LiveSessionId,
    ) -> StorageResult<SessionCardMeta> {
        if session.0 <= 0 {
            return Ok(SessionCardMeta::default());
        }
        self.with_conn(StorageErrorKind::Query, |c| c.query_row("SELECT COALESCE(label,''),COALESCE(pinned,0),COALESCE(color,''),COALESCE(note,''),COALESCE(auto_title,'') FROM sessions WHERE live_session_id=? ORDER BY id DESC LIMIT 1",[session.0],|r| Ok(SessionCardMeta { label:r.get(0)?,pinned:r.get::<_,i64>(1)?!=0,color:r.get(2)?,note:r.get(3)?,auto_title:r.get(4)? })).optional().map(Option::unwrap_or_default))
    }
    fn update_session_card_meta(
        &self,
        session: LiveSessionId,
        meta: SessionCardMeta,
    ) -> StorageResult<()> {
        self.with_conn(StorageErrorKind::Write, |c| c.execute("UPDATE sessions SET label=?,pinned=?,color=?,note=?,auto_title=?,updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![meta.label,meta.pinned,meta.color,meta.note,meta.auto_title,now(),session.0]).map(|_| ()))
    }
    fn end_session(&self, session: LiveSessionId, state: &str, reason: &str, ended_at: SystemTime) {
        let result = self.with_conn(StorageErrorKind::Write, |c| c.execute("UPDATE sessions SET state=COALESCE(NULLIF(?, ''),state),end_reason=COALESCE(NULLIF(?, ''),end_reason),ended_at=?,updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![state,reason,timestamp(ended_at)?,now(),session.0]).map(|_| ()));
        self.inner.notify(session, result);
    }
    fn clear_session_history(&self, session: LiveSessionId) -> StorageResult<()> {
        self.with_conn(StorageErrorKind::Write, |c| {
            let Some(id) = resolve(c,session,true)? else { return Ok(()); };
            let tx = c.transaction()?;
            if self.inner.fts { tx.execute("DELETE FROM messages_fts WHERE rowid IN (SELECT id FROM messages WHERE session_id=?)",[id])?; }
            for table in ["messages","events","approvals","attachments"] { tx.execute(&format!("DELETE FROM {table} WHERE session_id=?"),[id])?; }
            tx.execute("UPDATE sessions SET first_message=NULL,last_message=NULL,title=NULL,tags_json=NULL,summary=NULL,updated_at=? WHERE id=?",params![now(),id])?;
            tx.commit()
        })
    }
    fn store_event(&self, session: LiveSessionId, event: HistoryEvent) -> StorageResult<()> {
        let _barrier = self.inner.history.read().unwrap_or_else(|e| e.into_inner());
        self.with_conn(StorageErrorKind::Write, |c| {
            store_event(c, self.inner.fts, session, &event)
        })
    }
    fn store_approval_detected(&self, d: ApprovalDetected) {
        if d.sig.is_empty() {
            return;
        }
        let result = self.with_conn(StorageErrorKind::Write, |c| {
            let Some(id) = resolve(c,d.live_session_id,true)? else { return Ok(()); };
            let options = serde_json::to_string(&d.options).unwrap_or_else(|_| "null".into());
            c.execute("INSERT INTO approvals(session_id,sig,source,kind,provider,question,context,options_json,block,candidate_key,source_epoch,state,detected_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,'pending',?) ON CONFLICT(session_id,sig) DO UPDATE SET source=excluded.source,kind=excluded.kind,provider=excluded.provider,question=excluded.question,context=excluded.context,options_json=excluded.options_json,block=excluded.block,candidate_key=excluded.candidate_key,source_epoch=excluded.source_epoch,state='pending',detected_at=excluded.detected_at,resolved_at=NULL",params![id,d.sig,d.source,d.kind,d.provider,d.question,d.context,options,d.block,d.candidate_key,d.source_epoch.0 as i64,timestamp(d.detected_at.unwrap_or_else(SystemTime::now))?]).map(|_| ())
        });
        self.inner.notify(d.live_session_id, result);
    }
    fn store_approval_consumed(
        &self,
        session: LiveSessionId,
        sig: &str,
        selected_text: &str,
        resolved_at: SystemTime,
    ) {
        if sig.is_empty() {
            return;
        }
        let result = self.with_conn(StorageErrorKind::Write, |c| {
            let Some(id) = resolve(c,session,true)? else { return Ok(()); };
            c.execute("UPDATE approvals SET state='resolved',selected_text=?,resolved_at=? WHERE session_id=? AND sig=?",params![selected_text,timestamp(resolved_at)?,id,sig]).map(|_| ())
        });
        self.inner.notify(session, result);
    }
    fn approvals_by_live_session(
        &self,
        session: LiveSessionId,
        limit: i64,
        pending_only: bool,
    ) -> StorageResult<StoredRows<ApprovalRow>> {
        self.with_conn(StorageErrorKind::Query, |c| {
            let Some(id) = resolve(c, session, false)? else {
                return Ok(Some(Vec::new()));
            };
            approvals(
                c,
                if pending_only {
                    "a.session_id=? AND a.state='pending'"
                } else {
                    "a.session_id=?"
                },
                vec![id.into()],
                limit,
            )
            .map(Some)
        })
    }
    fn approvals_by_session_id(
        &self,
        session: DbSessionId,
        limit: i64,
        pending_only: bool,
    ) -> StorageResult<StoredRows<ApprovalRow>> {
        if session.0 <= 0 {
            return Ok(Some(Vec::new()));
        }
        self.with_conn(StorageErrorKind::Query, |c| {
            approvals(
                c,
                if pending_only {
                    "a.session_id=? AND a.state='pending'"
                } else {
                    "a.session_id=?"
                },
                vec![session.0.into()],
                limit,
            )
            .map(Some)
        })
    }
    fn latest_approval_of_kinds(
        &self,
        session: LiveSessionId,
        source: &str,
        kinds: &[String],
    ) -> StorageResult<Option<ApprovalRow>> {
        if kinds.is_empty() {
            return Ok(None);
        }
        self.with_conn(StorageErrorKind::Query, |c| {
            let Some(id) = resolve(c, session, true)? else {
                return Ok(None);
            };
            let mut args = vec![id.into(), source.to_owned().into()];
            args.extend(kinds.iter().cloned().map(rusqlite::types::Value::from));
            approvals(
                c,
                &format!(
                    "a.session_id=? AND a.source=? AND a.kind IN ({})",
                    placeholders(kinds.len())
                ),
                args,
                1,
            )
            .map(|r| r.into_iter().next())
        })
    }
    fn recent_approvals(
        &self,
        limit: i64,
        pending_only: bool,
    ) -> StorageResult<StoredRows<ApprovalRow>> {
        self.with_conn(StorageErrorKind::Query, |c| {
            approvals(
                c,
                if pending_only {
                    "a.state='pending'"
                } else {
                    ""
                },
                Vec::new(),
                limit,
            )
            .map(Some)
        })
    }
    fn chat_messages_by_live_session(
        &self,
        session: LiveSessionId,
        limit: i64,
    ) -> StorageResult<StoredRows<ChatMessage>> {
        self.with_conn(StorageErrorKind::Query, |c| {
            let Some(id) = resolve(c, session, false)? else {
                return Ok(None);
            };
            chat(c, id, session, limit).map(Some)
        })
    }
    fn chat_messages_by_session_id(
        &self,
        session: DbSessionId,
        limit: i64,
    ) -> StorageResult<StoredRows<ChatMessage>> {
        if session.0 <= 0 {
            return Ok(None);
        }
        self.with_conn(StorageErrorKind::Query, |c| {
            let live = c
                .query_row(
                    "SELECT live_session_id FROM sessions WHERE id=?",
                    [session.0],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?;
            match live {
                Some(live) => chat(c, session.0, LiveSessionId(live), limit).map(Some),
                None => Ok(None),
            }
        })
    }
    fn search_messages(&self, query: &str, limit: i64) -> StorageResult<StoredRows<SearchResult>> {
        if query.trim().is_empty() {
            return Ok(None);
        }
        self.with_conn(StorageErrorKind::Query, |c| {
            let limit = normalized_limit(limit, 100, 50);
            if self.inner.fts
                && let Ok(rows) = search(c, query.trim(), limit, true)
            {
                return Ok(nullable(rows));
            }
            search(c, query.trim(), limit, false).map(nullable)
        })
    }
    fn messages_mention_text(
        &self,
        session: LiveSessionId,
        variants: &[String],
    ) -> StorageResult<bool> {
        self.with_conn(StorageErrorKind::Query, |c| {
            let Some(id) = resolve(c,session,false)? else { return Ok(false); };
            for variant in variants.iter().filter(|v| !v.is_empty()) {
                let found = c.query_row("SELECT 1 FROM messages WHERE session_id=? AND role='user' AND (instr(COALESCE(raw_text,''),?)>0 OR instr(COALESCE(text,''),?)>0) LIMIT 1",params![id,variant,variant],|_| Ok(())).optional()?;
                if found.is_some() { return Ok(true); }
            } Ok(false)
        })
    }
    fn list_sessions(
        &self,
        limit: i64,
        include_archived: bool,
    ) -> StorageResult<StoredRows<SessionOverview>> {
        self.with_conn(StorageErrorKind::Query, |c| {
            overviews(
                c,
                &format!(
                    "{} ORDER BY {ACTIVITY} DESC,se.id DESC LIMIT ?",
                    if include_archived {
                        ""
                    } else {
                        "WHERE COALESCE(se.archived,0)=0"
                    }
                ),
                [normalized_limit(limit, 500, 100)],
            )
            .map(nullable)
        })
    }
    fn session_overview_by_live_session(
        &self,
        session: LiveSessionId,
    ) -> StorageResult<SessionOverview> {
        self.with_conn(StorageErrorKind::Query, |c| {
            match resolve(c, session, false)? {
                None => Ok(SessionOverview::default()),
                Some(id) => overview(c, id),
            }
        })
    }
    fn session_overview_by_session_id(
        &self,
        session: DbSessionId,
    ) -> StorageResult<SessionOverview> {
        if session.0 <= 0 {
            return Ok(SessionOverview::default());
        }
        self.with_conn(StorageErrorKind::Query, |c| overview(c, session.0))
    }
    fn update_session_meta(
        &self,
        session: LiveSessionId,
        title: &str,
        tags: &[String],
        summary: &str,
        archived: bool,
    ) -> StorageResult<SessionOverview> {
        self.with_conn(StorageErrorKind::Write, |c| {
            let Some(id)=resolve(c,session,true)? else { return Ok(SessionOverview::default()); };
            c.execute("UPDATE sessions SET title=?,tags_json=?,summary=?,archived=?,updated_at=? WHERE id=?",params![text::trim(title.trim(),160),serde_json::to_string(&text::tags(tags)).unwrap(),text::trim(summary.trim(),4000),archived,now(),id])?;
            overview(c,id)
        })
    }
    fn timeline_by_live_session(
        &self,
        session: LiveSessionId,
        limit: i64,
    ) -> StorageResult<StoredRows<TimelineEvent>> {
        self.with_conn(StorageErrorKind::Query,|c| {
            let Some(id)=resolve(c,session,false)? else { return Ok(None); };
            let mut rows=c.prepare("SELECT id,session_id,COALESCE(ts,''),type,payload_json FROM events WHERE session_id=? ORDER BY id DESC LIMIT ?")?.query_map(params![id,normalized_limit(limit,2000,400)],|r| Ok(TimelineEvent { id:r.get(0)?,session:DbSessionId(r.get(1)?),ts:r.get(2)?,r#type:r.get(3)?,payload:serde_json::from_str(&r.get::<_,String>(4)?).unwrap_or_default() }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows.reverse(); Ok(nullable(rows))
        })
    }
    fn usage_summary(&self) -> StorageResult<UsageSummary> {
        self.with_conn(StorageErrorKind::Query,|c| {
            let total_sessions=c.query_row("SELECT COUNT(*) FROM sessions",[],|r|r.get(0))?;
            let total_messages=c.query_row("SELECT COUNT(*) FROM messages",[],|r|r.get(0))?;
            let providers=c.prepare("SELECT COALESCE(se.provider,''),COALESCE(se.model,''),COUNT(DISTINCT se.id),COUNT(m.id),COALESCE(SUM(CASE WHEN m.role='user' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN m.role='ai' THEN 1 ELSE 0 END),0) FROM sessions se LEFT JOIN messages m ON m.session_id=se.id GROUP BY COALESCE(se.provider,''),COALESCE(se.model,'') ORDER BY COUNT(m.id) DESC,COUNT(DISTINCT se.id) DESC")?.query_map([],|r|Ok(UsageBucket{provider:r.get(0)?,model:r.get(1)?,sessions:r.get(2)?,messages:r.get(3)?,user_msgs:r.get(4)?,ai_msgs:r.get(5)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(UsageSummary{total_sessions,total_messages,providers:nullable(providers)})
        })
    }
    fn stale_sessions(
        &self,
        cutoff: SystemTime,
        limit: i64,
    ) -> StorageResult<StoredRows<SessionOverview>> {
        self.with_conn(StorageErrorKind::Query,|c|overviews(c,&format!("WHERE COALESCE(se.archived,0)=0 AND (se.ended_at IS NOT NULL OR se.state IN ('completed','error','disconnected','dismissed') OR COALESCE(NULLIF(se.last_output_at,''),NULLIF(se.started_at,''),se.created_at) < ?) ORDER BY {ACTIVITY} ASC,se.id ASC LIMIT ?"),params![timestamp(cutoff)?,normalized_limit(limit,500,100)]).map(nullable))
    }
    fn prune_older_than(&self, cutoff: SystemTime) -> StorageResult<()> {
        self.prune(cutoff)
    }
    fn prune_transcript_noise(&self) -> StorageResult<i64> {
        self.prune_noise()
    }
    fn reset_history(&self, preserve: &[LiveSessionId]) -> StorageResult<ResetResult> {
        self.reset(preserve)
    }
    fn history_generation(&self) -> HistoryGeneration {
        HistoryGeneration(self.inner.generation.load(Ordering::Acquire))
    }
    fn shutdown<'a>(
        &'a self,
        policy: ShutdownPolicy,
        cancellation: &'a HubShutdownCancellation,
    ) -> CoreFuture<'a, ShutdownReport> {
        Box::pin(async move {
            if matches!(policy, ShutdownPolicy::CompatibilityStop) {
                self.inner.stop.store(true, Ordering::Release);
            }
            self.begin_close();
            let timeout = match policy {
                ShutdownPolicy::CompatibilityStop => Duration::from_secs(6),
                ShutdownPolicy::Drain { timeout } => timeout,
            };
            let until = Instant::now() + timeout;
            while !self.inner.done.load(Ordering::Acquire) {
                if cancellation.token().is_cancelled() {
                    let mut r = self.shutdown_report();
                    r.cancelled = true;
                    return r;
                }
                if Instant::now() >= until {
                    let mut r = self.shutdown_report();
                    r.timed_out = true;
                    return r;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            let mut report = self.shutdown_report();
            if let Err(err) = self.finish_close() {
                report.error = Some(err);
            }
            report
        })
    }
}

pub(super) fn store_event(
    c: &mut Connection,
    fts: bool,
    live: LiveSessionId,
    event: &HistoryEvent,
) -> rusqlite::Result<()> {
    let Some(id) = resolve(c, live, true)? else {
        return Ok(());
    };
    let ts = text::value(event, "ts");
    let kind = text::value(event, "type");
    let kind = if kind.is_empty() { "event" } else { &kind };
    let payload = if kind == "pty_output" {
        String::new()
    } else {
        serde_json::to_string(&event.0).unwrap()
    };
    let tx = c.transaction()?;
    if kind != "pty_output" {
        tx.execute(
            "INSERT INTO events(session_id,ts,type,payload_json) VALUES (?,?,?,?)",
            params![id, ts, kind, payload],
        )?;
    }
    match kind {
        "user_input" => {
            let body = text::mask_secrets(&text::value(event, "text"))
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            if !body.trim().is_empty() {
                insert_message(&tx, fts, id, &ts, "user", "text", &body, &payload)?;
            }
        }
        "pty_output" => {
            let provider: Option<String> =
                tx.query_row("SELECT provider FROM sessions WHERE id=?", [id], |r| {
                    r.get(0)
                })?;
            if !matches!(provider.as_deref(), Some("claude" | "codex")) {
                let body = text::visible(&text::value(event, "text")).trim().to_owned();
                if !text::noise(&body) {
                    insert_message(&tx, fts, id, &ts, "ai", "text", &body, "")?;
                }
            }
        }
        "attach" => {
            let file = text::value(event, "filename");
            let path = text::value(event, "path");
            let body = if file.is_empty() { &path } else { &file };
            if !body.is_empty() {
                tx.execute(
                    "INSERT INTO attachments(session_id,ts,path,filename) VALUES (?,?,?,?)",
                    params![id, ts, path, file],
                )?;
                insert_message(&tx, fts, id, &ts, "system", "attach", body, &payload)?;
            }
        }
        _ => {}
    }
    match kind {
        "session_start" | "session_reattach" => {
            tx.execute("UPDATE sessions SET branch=COALESCE(NULLIF(?,''),branch),label=COALESCE(NULLIF(?,''),label),model=COALESCE(NULLIF(?,''),model),shell=COALESCE(NULLIF(?,''),shell),updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![text::value(event,"branch"),text::value(event,"label"),text::value(event,"model"),text::value(event,"shell"),now(),live.0])?;
        }
        "session_end" => {
            let current = now();
            tx.execute("UPDATE sessions SET state=COALESCE(NULLIF(?,''),state),end_reason=COALESCE(NULLIF(?,''),end_reason),ended_at=COALESCE(NULLIF(?,''),?),updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![text::value(event,"state"),text::value(event,"reason"),ts,current,current,live.0])?;
        }
        "user_input" => {
            let body = text::value(event, "text")
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            if !body.trim().is_empty() {
                let last = if body.bytes().all(|b| b.is_ascii_digit()) {
                    ""
                } else {
                    &body
                };
                tx.execute("UPDATE sessions SET first_message=CASE WHEN first_message IS NULL OR first_message='' THEN ? ELSE first_message END,last_message=COALESCE(NULLIF(?,''),last_message),updated_at=? WHERE live_session_id=? AND ended_at IS NULL",params![body,last,now(),live.0])?;
            }
        }
        _ => {}
    }
    tx.commit()
}
#[allow(clippy::too_many_arguments)]
fn insert_message(
    c: &Connection,
    fts: bool,
    id: i64,
    ts: &str,
    role: &str,
    kind: &str,
    body: &str,
    payload: &str,
) -> rusqlite::Result<()> {
    c.execute("INSERT INTO messages(session_id,ts,role,kind,text,raw_text,payload_json) VALUES (?,?,?,?,?,?,?)",params![id,ts,role,kind,body,body,payload])?;
    if fts
        && c.execute(
            "INSERT INTO messages_fts(rowid,text,raw_text) VALUES (?,?,?)",
            params![c.last_insert_rowid(), body, body],
        )
        .is_err()
    {
        // Match baseline soft-failure visibility without logging SQL or contents.
        eprintln!("sessionstore: search index update failed; message saved");
    }
    Ok(())
}
fn placeholders(count: usize) -> String {
    vec!["?"; count].join(",")
}

const APPROVAL_SELECT: &str = r#"SELECT a.id, a.session_id, se.live_session_id,
		COALESCE(NULLIF(a.provider, ''), COALESCE(se.provider, '')),
		COALESCE(se.cwd, ''), a.sig, COALESCE(a.source, ''), COALESCE(a.kind, ''),
		COALESCE(a.question, ''), COALESCE(a.context, ''), COALESCE(a.options_json, ''),
		COALESCE(a.block, ''), COALESCE(a.candidate_key, ''), COALESCE(a.source_epoch, 0),
		COALESCE(a.selected_text, ''), a.state, COALESCE(a.detected_at, ''),
		COALESCE(a.resolved_at, '')
	FROM approvals a JOIN sessions se ON se.id = a.session_id"#;
const OVERVIEW_SELECT: &str = r#"SELECT
			se.id, se.live_session_id, COALESCE(se.provider, ''), COALESCE(se.display_name, ''), COALESCE(se.cwd, ''),
			COALESCE(se.branch, ''), COALESCE(se.label, ''), COALESCE(se.model, ''), COALESCE(se.route, ''),
			COALESCE(se.shell, ''), COALESCE(se.state, ''), COALESCE(se.started_at, ''), COALESCE(se.last_output_at, ''),
			COALESCE(se.ended_at, ''), COALESCE(se.first_message, ''), COALESCE(se.last_message, ''),
			COALESCE(se.end_reason, ''), COALESCE(se.title, ''), COALESCE(se.tags_json, '[]'), COALESCE(se.summary, ''),
			COALESCE(se.archived, 0), COALESCE(se.log_path, ''), COALESCE(se.jsonl_path, ''),
			COALESCE(se.parent_session_id, 0), COALESCE(se.role, ''), COALESCE(se.auto, 0),
			COALESCE(se.depth, 0), COALESCE(se.orchestration_id, ''), COALESCE(se.board_path, ''),
			COALESCE(se.worktree_branch, ''), COALESCE(se.subscription_id, ''),
			(SELECT COUNT(*) FROM messages m WHERE m.session_id=se.id),
			(SELECT COUNT(*) FROM events e WHERE e.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id AND a.state='pending')
		FROM sessions se"#;
const ACTIVITY: &str = "COALESCE(NULLIF(se.last_output_at,''),NULLIF(se.ended_at,''),NULLIF(se.started_at,''),se.updated_at)";
fn approvals(
    c: &Connection,
    where_clause: &str,
    mut args: Vec<rusqlite::types::Value>,
    limit: i64,
) -> rusqlite::Result<Vec<ApprovalRow>> {
    args.push(if limit <= 0 { 100 } else { limit.min(500) }.into());
    let sql = format!(
        "{APPROVAL_SELECT} {} ORDER BY a.detected_at DESC,a.id DESC LIMIT ?",
        if where_clause.is_empty() {
            String::new()
        } else {
            format!("WHERE {where_clause}")
        }
    );
    c.prepare(&sql)?
        .query_map(params_from_iter(args), |r| {
            Ok(ApprovalRow {
                id: r.get(0)?,
                session_db_id: DbSessionId(r.get(1)?),
                live_session_id: LiveSessionId(r.get(2)?),
                provider: r.get(3)?,
                cwd: r.get(4)?,
                sig: r.get(5)?,
                source: r.get(6)?,
                kind: r.get(7)?,
                question: r.get(8)?,
                context: r.get(9)?,
                options: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or_default(),
                block: r.get(11)?,
                candidate_key: r.get(12)?,
                source_epoch: ApprovalSourceEpoch(r.get::<_, i64>(13)?.max(0) as u64),
                selected_text: r.get(14)?,
                state: r.get(15)?,
                detected_at: r.get(16)?,
                resolved_at: r.get(17)?,
            })
        })?
        .collect()
}
fn chat(
    c: &Connection,
    id: i64,
    live: LiveSessionId,
    limit: i64,
) -> rusqlite::Result<Vec<ChatMessage>> {
    let mut out=c.prepare("SELECT id,ts,role,kind,COALESCE(text,''),COALESCE(raw_text,''),COALESCE(payload_json,'') FROM messages WHERE session_id=? ORDER BY id DESC LIMIT ?")?.query_map(params![id,normalized_limit(limit,1000,400)],|r|{
        let payload:String=r.get(6)?;let mut msg=ChatMessage{id:r.get(0)?,session_id:DbSessionId(id),live_session_id:live,ts:r.get(1)?,role:r.get(2)?,kind:r.get(3)?,normalized_text:r.get(4)?,raw_text:r.get(5)?,..Default::default()};
        if msg.raw_text.is_empty(){msg.raw_text=msg.normalized_text.clone();}
        if let Ok(meta)=serde_json::from_str::<JsonObject>(&payload){
            if msg.kind=="attach" { let e=HistoryEvent(meta.clone());msg.attachments=vec![AttachmentRef{path:text::value(&e,"path"),filename:text::value(&e,"filename"),kind:"file".into()}]; }
            msg.meta=meta;
        } Ok(msg)
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    out.reverse();
    Ok(text::coalesce(out))
}
fn search(
    c: &Connection,
    query: &str,
    limit: i64,
    fts: bool,
) -> rusqlite::Result<Vec<SearchResult>> {
    let common = "SELECT m.id,m.session_id,se.live_session_id,se.provider,se.cwd,se.branch,se.model,se.state,se.started_at,m.ts,m.role,m.kind,COALESCE(m.text,''),";
    let (sql, query) = if fts {
        (
            format!(
                "{common} snippet(messages_fts,0,'','','...',16) FROM messages_fts JOIN messages m ON m.id=messages_fts.rowid JOIN sessions se ON se.id=m.session_id WHERE messages_fts MATCH ? ORDER BY rank LIMIT ?"
            ),
            query
                .split_whitespace()
                .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND "),
        )
    } else {
        (
            format!(
                "{common} COALESCE(m.text,'') FROM messages m JOIN sessions se ON se.id=m.session_id WHERE m.text LIKE ? ESCAPE '\\' ORDER BY m.id DESC LIMIT ?"
            ),
            format!(
                "%{}%",
                query
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            ),
        )
    };
    c.prepare(&sql)?
        .query_map(params![query, limit], |r| {
            let mut out = SearchResult {
                message_id: r.get(0)?,
                session_db_id: DbSessionId(r.get(1)?),
                live_session_id: LiveSessionId(r.get(2)?),
                provider: r.get(3)?,
                cwd: r.get(4)?,
                branch: r.get(5)?,
                model: r.get(6)?,
                state: r.get(7)?,
                started_at: r.get(8)?,
                ts: r.get(9)?,
                role: r.get(10)?,
                kind: r.get(11)?,
                text: r.get(12)?,
                snippet: r.get(13)?,
            };
            if out.snippet.is_empty() {
                out.snippet = out.text.clone();
            }
            if out.snippet.chars().count() > 240 {
                out.snippet = text::trim(&out.snippet, 240) + "...";
            }
            Ok(out)
        })?
        .collect()
}
fn overview(c: &Connection, id: i64) -> rusqlite::Result<SessionOverview> {
    Ok(overviews(c, "WHERE se.id=?", [id])?
        .into_iter()
        .next()
        .unwrap_or_default())
}
fn overviews(
    c: &Connection,
    suffix: &str,
    args: impl rusqlite::Params,
) -> rusqlite::Result<Vec<SessionOverview>> {
    c.prepare(&format!("{OVERVIEW_SELECT} {suffix}"))?
        .query_map(args, scan_overview)?
        .collect()
}
fn scan_overview(r: &Row<'_>) -> rusqlite::Result<SessionOverview> {
    let tags = serde_json::from_str::<Vec<String>>(&r.get::<_, String>(18)?).unwrap_or_default();
    Ok(SessionOverview {
        id: DbSessionId(r.get(0)?),
        live_session_id: LiveSessionId(r.get(1)?),
        provider: r.get(2)?,
        display: r.get(3)?,
        cwd: r.get(4)?,
        branch: r.get(5)?,
        label: r.get(6)?,
        model: r.get(7)?,
        route: r.get(8)?,
        shell: r.get(9)?,
        state: r.get(10)?,
        started_at: r.get(11)?,
        last_output_at: r.get(12)?,
        ended_at: r.get(13)?,
        first_message: r.get(14)?,
        last_message: r.get(15)?,
        end_reason: r.get(16)?,
        title: r.get(17)?,
        tags: text::tags(&tags),
        summary: r.get(19)?,
        archived: r.get::<_, i64>(20)? != 0,
        log_path: r.get(21)?,
        jsonl_path: r.get(22)?,
        parent_session_id: LiveSessionId(r.get(23)?),
        role: r.get(24)?,
        auto: r.get::<_, i64>(25)? != 0,
        depth: r.get(26)?,
        orchestration_id: OrchestrationId(r.get(27)?),
        board_path: r.get(28)?,
        worktree_branch: r.get(29)?,
        subscription_id: r.get(30)?,
        message_count: r.get(31)?,
        event_count: r.get(32)?,
        approval_count: r.get(33)?,
        pending_count: r.get(34)?,
    })
}
