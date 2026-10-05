use super::*;
#[derive(Clone, Default)]
pub(super) struct Writer {
    pub role: String,
    pub id: LiveSessionId,
}
#[derive(Clone)]
struct Marker {
    role: String,
    id: LiveSessionId,
    writer: Writer,
}
fn session_id(fields: &[&str]) -> LiveSessionId {
    fields
        .iter()
        .find_map(|field| {
            field
                .strip_prefix("session=")?
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
        })
        .map(LiveSessionId)
        .unwrap_or(LiveSessionId(0))
}
fn role(text: &str) -> String {
    child_launch::prompt::normalize_role(text).unwrap_or_default()
}
fn done_markers(text: &str) -> Vec<Marker> {
    let mut result = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(text) = line
            .strip_prefix("## DONE ")
            .or_else(|| line.strip_prefix("## SUCCESS "))
        else {
            continue;
        };
        let fields = text.split_whitespace().collect::<Vec<_>>();
        let Some(first) = fields.first() else {
            continue;
        };
        let role = role(first);
        if role.is_empty() {
            continue;
        }
        let id = session_id(&fields[1..]);
        if !result
            .iter()
            .any(|marker: &Marker| marker.id == id && marker.role == role)
        {
            result.push(Marker {
                role,
                id,
                writer: Default::default(),
            });
        }
    }
    result
}
fn questions(text: &str, mut writer: Writer) -> (Vec<Marker>, Writer) {
    let mut result = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(text) = line.strip_prefix("## ") else {
            continue;
        };
        if text.starts_with("DONE ") || text.starts_with("SUCCESS ") {
            continue;
        }
        let question = text.strip_prefix("QUESTION ");
        let fields = question
            .unwrap_or(text)
            .split_whitespace()
            .collect::<Vec<_>>();
        let Some(first) = fields.first() else {
            continue;
        };
        let role = role(first);
        let id = session_id(&fields[1..]);
        if question.is_some() {
            if !role.is_empty() && id.0 > 0 {
                result.push(Marker {
                    role,
                    id,
                    writer: writer.clone(),
                });
            }
        } else {
            writer = Writer { role, id };
        }
    }
    (result, writer)
}
fn last_writer(text: &str) -> Writer {
    questions(text, Writer::default()).1
}
fn unique_child(board: &Board, role: &str) -> Option<LiveSessionId> {
    let mut ids = board
        .children
        .iter()
        .filter_map(|(id, c)| (c.registration.role == role).then_some(*id));
    let first = ids.next()?;
    ids.next().is_none().then_some(first)
}
fn writer_id(board: &Board, writer: &Writer) -> Option<LiveSessionId> {
    if writer.id.0 > 0 && board.sessions.contains_key(&writer.id) {
        Some(writer.id)
    } else {
        unique_child(board, &writer.role)
    }
}
fn authorize_question(board: &Board, marker: &Marker) -> bool {
    writer_id(board, &marker.writer) == Some(marker.id)
        && board
            .children
            .get(&marker.id)
            .is_some_and(|child| child.registration.role == marker.role)
}
fn read(
    path: &Path,
    old: Option<(u64, std::time::SystemTime)>,
) -> Option<(String, (u64, std::time::SystemTime))> {
    let metadata = std::fs::metadata(path).ok()?;
    let stamp = (metadata.len(), metadata.modified().ok()?);
    if Some(stamp) == old || metadata.is_dir() {
        return None;
    }
    let text = String::from_utf8_lossy(&std::fs::read(path).ok()?).into_owned();
    Some((text, stamp))
}
fn appended(text: &str, cursor: usize) -> &str {
    text.get(cursor..).unwrap_or("")
}
struct Notice {
    parent: LiveSessionId,
    text: String,
    actionable: bool,
}
struct Rejection {
    path: PathBuf,
    writer: LiveSessionId,
    claimed: LiveSessionId,
    role: String,
}
pub(super) async fn poll(owner: &OrchestrationProgram) -> Result<(), SessionError> {
    let core = owner.core()?;
    let now = Timestamp::now();
    let files = {
        let state = lock(&owner.state);
        state
            .boards
            .iter()
            .filter(|(id, _)| !state.relay_boards.contains(*id))
            .map(|(id, board)| {
                (
                    id.clone(),
                    board.path.clone(),
                    board.stamp,
                    board.cursor,
                    board.writer.clone(),
                    board
                        .children
                        .iter()
                        .map(|(id, child)| {
                            (
                                *id,
                                child_launch::prompt::child_progress_path(
                                    &board.path,
                                    &id.0.to_string(),
                                ),
                                child.stamp,
                                child.cursor,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut notices = Vec::new();
    let mut rejected = Vec::new();
    let mut completed = Vec::new();
    for (id, path, stamp, cursor, writer, children) in files {
        crate::profile::subscriptions::check_path(&owner.deps.paths, &path)
            .map_err(|_| SessionError::InvalidRequest("board poll escaped trial".into()))?;
        if let Some((text, stamp)) = read(&path, stamp) {
            let done = done_markers(&text);
            let (question, next_writer) = questions(appended(&text, cursor), writer);
            let last = last_writer(&text);
            let mut state = lock(&owner.state);
            let board = state.boards.get_mut(&id).expect("board owned");
            board.stamp = Some(stamp);
            board.cursor = text.len();
            board.writer = next_writer;
            let parent = board.conductor();
            let writer = writer_id(board, &last);
            let mut authorized_done = false;
            let mut authorized_question = false;
            for marker in done {
                let claimed = if marker.id.0 > 0 {
                    Some(marker.id)
                } else {
                    unique_child(board, &marker.role)
                };
                if let Some(claimed) = claimed.filter(|claimed| Some(*claimed) == writer)
                    && let Some(child) = board.children.get_mut(&claimed)
                    && !child.done
                {
                    child.done = true;
                    child.last_board_write = now;
                    authorized_done = true;
                    completed.push(claimed);
                    if let Some(parent) = parent {
                        notices.push(Notice {
                            parent,
                            text: format!(
                                "\n[orchestration] child complete role={} session={} progress={}\n",
                                child.registration.role,
                                claimed.0,
                                child_launch::prompt::child_progress_path(
                                    &path,
                                    &claimed.0.to_string()
                                )
                                .display()
                            ),
                            actionable: true,
                        });
                    }
                }
            }
            for marker in question {
                if authorize_question(board, &marker) {
                    authorized_question = true;
                    if let Some(parent) = parent {
                        notices.push(Notice {
                            parent,
                            text: format!(
                                "\n[orchestration] question from role={} session={} progress={}\n",
                                marker.role,
                                marker.id.0,
                                path.display()
                            ),
                            actionable: true,
                        });
                    }
                } else {
                    rejected.push(Rejection {
                        path: path.clone(),
                        writer: marker.writer.id,
                        claimed: marker.id,
                        role: marker.role,
                    });
                }
            }
            if !authorized_done
                && !authorized_question
                && last.role != "hub"
                && let Some(parent) = parent.filter(|p| Some(*p) != writer)
            {
                notices.push(Notice {
                    parent,
                    text: format!(
                        "\n[orchestration] board updated by {}: {}\n",
                        if last.role.is_empty() {
                            "unknown"
                        } else {
                            &last.role
                        },
                        path.display()
                    ),
                    actionable: false,
                });
            }
        }
        for (child_id, child_path, stamp, cursor) in children {
            crate::profile::subscriptions::check_path(&owner.deps.paths, &child_path)
                .map_err(|_| SessionError::InvalidRequest("child progress escaped trial".into()))?;
            if let Some((text, stamp)) = read(&child_path, stamp) {
                let done = done_markers(&text);
                let (question, _) = questions(
                    appended(&text, cursor),
                    Writer {
                        role: String::new(),
                        id: child_id,
                    },
                );
                let mut state = lock(&owner.state);
                let board = state.boards.get_mut(&id).expect("board owned");
                let child = board.children.get_mut(&child_id).expect("child owned");
                child.stamp = Some(stamp);
                child.cursor = text.len();
                child.last_board_write = now;
                let became_done = !child.done
                    && done
                        .iter()
                        .any(|marker| marker.id.0 == 0 || marker.id == child_id);
                if became_done {
                    child.done = true;
                    completed.push(child_id);
                }
                let mut authorized = false;
                for marker in question {
                    if marker.id == child_id && marker.role == child.registration.role {
                        authorized = true;
                        notices.push(Notice {
                            parent: child.registration.parent.session,
                            text: format!(
                                "\n[orchestration] question from role={} session={} progress={}\n",
                                child.registration.role,
                                child_id.0,
                                child_path.display()
                            ),
                            actionable: true,
                        });
                    } else {
                        rejected.push(Rejection {
                            path: path.clone(),
                            writer: child_id,
                            claimed: marker.id,
                            role: marker.role,
                        });
                    }
                }
                if child.registration.parent.session.0 > 0 && (became_done || !authorized) {
                    let text = if became_done {
                        format!(
                            "\n[orchestration] child complete role={} session={} progress={}\n",
                            child.registration.role,
                            child_id.0,
                            child_path.display()
                        )
                    } else {
                        format!(
                            "\n[orchestration] progress updated by {} (session={}): {}\n",
                            child.registration.role,
                            child_id.0,
                            child_path.display()
                        )
                    };
                    notices.push(Notice {
                        parent: child.registration.parent.session,
                        text,
                        actionable: became_done,
                    });
                }
            }
        }
        // Every notice belongs to this board even if it was created from a
        // child's own file. Drain before scanning the next board.
        for notice in notices.drain(..) {
            if notice.actionable {
                owner.queue_event(notice.parent, &id, notice.text).await?;
            } else {
                owner
                    .progress_notice(notice.parent, &id, notice.text)
                    .await?;
            }
        }
        for rejection in rejected.drain(..) {
            owner
                .boards
                .append(
                    &rejection.path,
                    "hub",
                    &format!(
                        "QUESTION rejected: source_session={} claimed_role={} claimed_session={}\n",
                        rejection.writer.0, rejection.role, rejection.claimed.0
                    ),
                    now,
                )
                .map_err(|e| SessionError::Transport(e.to_string()))?;
        }
    }
    for child in completed {
        if let Some(details) = core.details(child) {
            owner
                .apply_effects(core.mark_orchestration_child_state(details.binding, "done", now)?)
                .await?;
        }
    }
    Ok(())
}
