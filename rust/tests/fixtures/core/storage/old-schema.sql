-- Exact base schema from Go 21d0bc7 internal/sessionstore/store.go.
CREATE TABLE IF NOT EXISTS sessions (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			live_session_id INTEGER NOT NULL,
			provider TEXT,
			display_name TEXT,
			cwd TEXT,
			branch TEXT,
			label TEXT,
			model TEXT,
			route TEXT,
			shell TEXT,
			state TEXT,
			started_at TEXT,
			last_output_at TEXT,
			ended_at TEXT,
			log_path TEXT,
			jsonl_path TEXT UNIQUE,
			first_message TEXT,
			last_message TEXT,
			end_reason TEXT,
			created_at TEXT NOT NULL DEFAULT (datetime('now')),
			updated_at TEXT NOT NULL DEFAULT (datetime('now'))
		);

CREATE TABLE IF NOT EXISTS events (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			ts TEXT,
			type TEXT NOT NULL,
			payload_json TEXT NOT NULL
		);

CREATE TABLE IF NOT EXISTS messages (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			ts TEXT,
			role TEXT NOT NULL,
			kind TEXT NOT NULL,
			text TEXT,
			raw_text TEXT,
			payload_json TEXT
		);

CREATE TABLE IF NOT EXISTS approvals (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			sig TEXT NOT NULL,
			source TEXT,
			kind TEXT,
			question TEXT,
			context TEXT,
			options_json TEXT,
			selected_text TEXT,
			state TEXT NOT NULL,
			detected_at TEXT,
			resolved_at TEXT,
			UNIQUE(session_id, sig)
		);

CREATE TABLE IF NOT EXISTS attachments (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			ts TEXT,
			path TEXT,
			filename TEXT,
			mime TEXT,
			size INTEGER
		);

CREATE INDEX IF NOT EXISTS idx_sessions_live ON sessions(live_session_id);

CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);

CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id);

CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, id);

CREATE INDEX IF NOT EXISTS idx_approvals_state ON approvals(state, detected_at);
INSERT INTO sessions(id,live_session_id,provider,jsonl_path,state) VALUES (1,7,'copilot','synthetic-old.jsonl','standby');
INSERT INTO approvals(session_id,sig,state,detected_at) VALUES (1,'legacy','resolved','2020-01-01T00:00:00Z');
