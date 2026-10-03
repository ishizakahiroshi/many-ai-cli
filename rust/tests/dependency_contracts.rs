#[test]
fn bundled_sqlite_has_external_content_fts5() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let enabled: bool = db
        .query_row("SELECT sqlite_compileoption_used('ENABLE_FTS5')", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(enabled);
    db.execute_batch("CREATE TABLE messages(id INTEGER PRIMARY KEY, text TEXT NOT NULL); CREATE VIRTUAL TABLE messages_fts USING fts5(text,content='messages',content_rowid='id'); INSERT INTO messages VALUES(1,'synthetic searchable message'); INSERT INTO messages_fts(rowid,text) SELECT id,text FROM messages;").unwrap();
    let text: String = db.query_row("SELECT messages.text FROM messages_fts JOIN messages ON messages.id=messages_fts.rowid WHERE messages_fts MATCH 'searchable'", [], |r|r.get(0)).unwrap();
    assert_eq!(text, "synthetic searchable message");
    assert_eq!(rusqlite::version(), "3.53.2");
}
