//! The executable wrapper caller: registration precedes provider exec, IO runs
//! independently of network backpressure, and only completed writes are ACKed.
use super::{
    input::{InputWatermarks, SharedWatermarks, write_all, write_input},
    transport::{HubConnector, HubSocket},
};
use crate::{
    process::{
        Cancellation, ProcessPlan,
        pty::{PtyExit, PtyFactory, PtySession, PtySize},
    },
    proto::{Message, core::SPAWN_PROOF_ENV},
    terminal::replay::ReplayBuffer,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

/// Deliberately no Debug: registration contains the Hub token, process.env may
/// contain credentials, and initial_proof is one-use internal correlation.
pub struct RawLogPolicy {
    pub paths: crate::config::RuntimePaths,
    pub max_bytes: i64,
}
pub struct WrapperOptions {
    pub registration: Message,
    pub raw_log: Option<RawLogPolicy>,
    pub process: ProcessPlan,
    pub initial_proof: Option<String>,
    pub reconnect_grace: Duration,
    pub reconnect_interval: Duration,
    pub write_timeout: Duration,
    pub auto_shutdown: bool,
    pub login_mode: bool,
    pub output_capacity: usize,
    pub input_capacity: usize,
    pub drain_timeout: Duration,
}
impl WrapperOptions {
    pub fn new(registration: Message, process: ProcessPlan) -> Self {
        Self {
            registration,
            raw_log: None,
            process,
            initial_proof: None,
            reconnect_grace: Duration::from_secs(3600),
            reconnect_interval: Duration::from_secs(2),
            write_timeout: Duration::from_secs(5),
            auto_shutdown: false,
            login_mode: false,
            output_capacity: 1024,
            input_capacity: 64,
            drain_timeout: Duration::from_secs(2),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrapperResult {
    pub session_id: i64,
    pub exit: PtyExit,
    pub reconnects: u64,
    pub output_bytes: i64,
}
#[derive(Clone)]
struct OutputChunk {
    bytes: Vec<u8>,
    end: i64,
}
struct InputFrame {
    seq: i64,
    data: Vec<u8>,
    attach: bool,
}
enum InputResult {
    Ack(i64),
    Failed,
}
struct Tasks {
    pty: Arc<dyn PtySession>,
    reader: JoinHandle<()>,
    input: JoinHandle<()>,
}
impl Drop for Tasks {
    fn drop(&mut self) {
        self.pty.close();
        self.reader.abort();
        self.input.abort();
    }
}
fn poison() -> io::Error {
    io::Error::other("wrapper state lock poisoned")
}
async fn send(socket: &mut dyn HubSocket, frame: &Message, timeout: Duration) -> io::Result<()> {
    tokio::time::timeout(timeout, socket.send(frame))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "wrapper Hub write timed out"))?
}
/// Registration-dependent hooks and prompt preparation execute through this
/// callback, while ownership is still entirely local and before provider exec.
pub async fn run<F>(
    mut options: WrapperOptions,
    connector: &dyn HubConnector,
    factory: &dyn PtyFactory,
    cancel: &Cancellation,
    prepare: F,
) -> io::Result<WrapperResult>
where
    F: FnOnce(&Message, &mut ProcessPlan) -> io::Result<()>,
{
    if options.write_timeout.is_zero()
        || options.reconnect_interval.is_zero()
        || options.drain_timeout.is_zero()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "wrapper timeouts must be positive",
        ));
    }
    options.registration.r#type = "register".into();
    options.registration.role = "wrapper".into();
    let (mut socket, mut registered) = super::transport::register(
        connector,
        &options.registration,
        options.initial_proof.as_deref(),
        options.write_timeout,
        cancel,
    )
    .await?;
    if registered.started_at.is_empty() {
        registered.started_at =
            chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    }
    let mut raw_log = if let Some(policy) = &options.raw_log {
        let fallback = crate::terminal::journal::session_log_paths(
            &policy.paths,
            crate::proto::core::LiveSessionId(registered.session_id),
            &options.registration.provider,
            &options.registration.cwd,
            &registered.started_at,
        )
        .map_err(|_| io::Error::other("cannot determine wrapper log paths"))?;
        let path = if registered.log_path.is_empty() {
            fallback.raw
        } else {
            std::path::PathBuf::from(&registered.log_path)
        };
        if policy.paths.is_trial()
            && !path.starts_with(policy.paths.resource(crate::config::Resource::Logs))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "wrapper raw log escapes selected runtime root",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("wrapper log path has no parent"))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| io::Error::other("wrapper log path has no filename"))?;
        let opened = (|| {
            let directory = crate::files::safe_fs::Dir::open_or_create_private_components(parent)?;
            directory.replace(name, &[], 0o600)?;
            directory.open_file(name, true)
        })();
        registered.log_path = path.to_string_lossy().into_owned();
        if registered.jsonl_path.is_empty() {
            registered.jsonl_path = path.with_extension("jsonl").to_string_lossy().into_owned();
        }
        match opened {
            Ok(file) => Some((file, policy.max_bytes, 0i64)),
            Err(_) => {
                eprintln!("wrapper: opted-in raw session log could not be opened");
                None
            }
        }
    } else {
        None
    };
    // Proof is never carried to reattach or provider children.
    options.initial_proof.take();
    if let Err(error) = prepare(&registered, &mut options.process) {
        let frame = Message {
            r#type: "session_end".into(),
            session_id: registered.session_id,
            state: "error".into(),
            exit_code: 1,
            reason: if error.kind() == io::ErrorKind::NotFound {
                "exec_not_found".into()
            } else {
                String::new()
            },
            ..Default::default()
        };
        let _ = send(socket.as_mut(), &frame, options.write_timeout).await;
        return Err(error);
    }
    super::startup::strip_provider_environment(&mut options.process);
    options.process.env.insert(SPAWN_PROOF_ENV.into(), None);
    let proof_keys: Vec<_> = options
        .process
        .env
        .keys()
        .filter(|key| key.to_string_lossy().eq_ignore_ascii_case(SPAWN_PROOF_ENV))
        .cloned()
        .collect();
    for key in proof_keys {
        options.process.env.insert(key, None);
    }
    options.process.env.insert(
        "MANY_AI_CLI_SESSION_ID".into(),
        Some(registered.session_id.to_string().into()),
    );
    options.process.env.insert(
        "MANY_AI_CLI_HUB_TOKEN".into(),
        Some(options.registration.token.clone().into()),
    );
    let size = PtySize::from_wire(registered.cols, registered.rows).unwrap_or_default();
    let pty = match factory.spawn(&options.process, size) {
        Ok(pty) => pty,
        Err(error) => {
            let mut socket = socket;
            let end = Message {
                r#type: "session_end".into(),
                session_id: registered.session_id,
                state: "error".into(),
                exit_code: 1,
                reason: if error.kind() == io::ErrorKind::NotFound {
                    "exec_not_found".into()
                } else {
                    String::new()
                },
                ..Default::default()
            };
            let _ = send(socket.as_mut(), &end, options.write_timeout).await;
            return Err(error);
        }
    };
    let replay = Arc::new(Mutex::new(ReplayBuffer::default()));
    let marks: SharedWatermarks = Arc::new(Mutex::new(InputWatermarks::default()));
    let (output_tx, mut output) = broadcast::channel::<OutputChunk>(options.output_capacity.max(1));
    let (done_tx, mut reader_done) = watch::channel(false);
    let reader_pty = pty.clone();
    let reader_replay = replay.clone();
    let login_mode = options.login_mode;
    let reader = tokio::spawn(async move {
        let mut bytes = vec![0u8; 4096];
        let mut normalize = super::output::OutputNormalizer::new(cfg!(windows));
        let mut login = super::output::LoginCompletion::default();
        let mut emit = |bytes: Vec<u8>| {
            if bytes.is_empty() {
                return;
            }
            if let Some((file, limit, written)) = &mut raw_log
                && (*limit <= 0 || *written < *limit)
            {
                use std::io::Write;
                // The source checks the cap before each complete chunk.
                // A raw-log write failure never blocks provider progress.
                let _ = file.write_all(&bytes);
                *written = written.saturating_add(bytes.len() as i64);
            }
            let Ok(mut replay) = reader_replay.lock() else {
                return;
            };
            replay.append(&bytes);
            let _ = output_tx.send(OutputChunk {
                bytes,
                end: replay.total(),
            });
        };
        loop {
            let n = match reader_pty.read(&mut bytes).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    reader_pty.close();
                    break;
                }
            };
            let bytes = normalize.push(&bytes[..n]);
            if login_mode && login.push(&bytes) {
                let pty = reader_pty.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(crate::process::pty::CLOSE_GRACE).await;
                    pty.close();
                });
            }
            emit(bytes);
        }
        emit(normalize.finish());
        done_tx.send_replace(true);
    });
    let (input_tx, mut inputs) = mpsc::channel::<InputFrame>(options.input_capacity.max(1));
    let (ack_tx, mut acks) = mpsc::channel(options.input_capacity.max(1));
    let input_pty = pty.clone();
    let input_marks = marks.clone();
    let provider = options.registration.provider.clone();
    let input = tokio::spawn(async move {
        while let Some(frame) = inputs.recv().await {
            let duplicate = input_marks
                .lock()
                .is_ok_and(|marks| marks.duplicate(frame.seq));
            let result = if duplicate {
                Ok(())
            } else if frame.attach {
                let result = write_all(input_pty.as_ref(), &frame.data).await;
                if result.is_ok() {
                    crate::logging::input_probe::wrapper_write(&provider, &frame.data);
                }
                result
            } else {
                write_input(input_pty.as_ref(), &provider, &frame.data).await
            };
            if result.is_err() {
                // Go logs the failed input without acknowledging it or killing
                // the session. Output/resize/later messages remain live.
                if ack_tx.send(InputResult::Failed).await.is_err() {
                    break;
                }
                continue;
            }
            if frame.seq > 0 {
                if let Ok(mut marks) = input_marks.lock() {
                    marks.processed(frame.seq);
                }
                if ack_tx.send(InputResult::Ack(frame.seq)).await.is_err() {
                    break;
                }
            }
        }
    });
    let _tasks = Tasks {
        pty: pty.clone(),
        reader,
        input,
    };
    let mut socket = Some(socket);
    let mut session_id = registered.session_id;
    let mut reconnects = 0;
    let mut last_reattach = None::<tokio::time::Instant>;
    let mut sent_offset = 0i64;
    let mut queued = VecDeque::new();
    let mut output_open = true;
    let mut intentional = false;
    let mut transport_fault = false;
    let exit;
    loop {
        if cancel.is_cancelled() {
            pty.close();
            exit = pty.wait().await?;
            break;
        }
        if socket.is_none() {
            let recent = last_reattach.is_some_and(|at| at.elapsed() < Duration::from_secs(10));
            if (!intentional
                && !transport_fault
                && !recent
                && (connector.probe().await || options.auto_shutdown))
                || options.reconnect_grace.is_zero()
            {
                pty.close();
                exit = pty.wait().await?;
                break;
            }
            let until = tokio::time::Instant::now() + options.reconnect_grace;
            let reattached = loop {
                tokio::select! {
                    _ = cancel.cancelled() => { pty.close(); break None; },
                    _ = pty.wait() => break None,
                    _ = tokio::time::sleep_until(until) => { pty.close(); break None; },
                    _ = tokio::time::sleep(options.reconnect_interval) => {},
                }
                let (bytes, total) = replay.lock().map_err(|_| poison())?.snapshot();
                let watermark = marks.lock().map_err(|_| poison())?.high_watermark();
                let mut frame = options.registration.clone();
                frame.r#type = "reattach".into();
                frame.session_id = session_id;
                frame.started_at = registered.started_at.clone();
                frame.log_path = registered.log_path.clone();
                frame.jsonl_path = registered.jsonl_path.clone();
                frame.replay_b64 = STANDARD.encode(bytes);
                frame.pty_bytes = total;
                frame.input_seq_high_watermark = watermark;
                let attempt = async {
                    let mut next = connector.connect(None).await?;
                    send(next.as_mut(), &frame, options.write_timeout).await?;
                    let mut pending = VecDeque::new();
                    loop {
                        let response = next.receive().await?;
                        match response.r#type.as_str() {
                            "reattach_reject" => return Ok(None),
                            "reattach_ack" if response.session_id > 0 => {
                                return Ok(Some((next, response.session_id, pending)));
                            }
                            "reattach_ack" => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "invalid reattach session ID",
                                ));
                            }
                            _ if pending.len() < 256 => pending.push_back(response),
                            _ => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "reattach pending frame limit exceeded",
                                ));
                            }
                        }
                    }
                };
                let attempt_until =
                    (tokio::time::Instant::now() + options.write_timeout).min(until);
                let result = tokio::select! {
                    _ = cancel.cancelled() => { pty.close(); break None; },
                    _ = pty.wait() => break None,
                    result = tokio::time::timeout_at(attempt_until, attempt) => result,
                };
                match result {
                    Ok(Ok(Some((next, sid, pending)))) => {
                        sent_offset = total;
                        break Some((next, sid, pending));
                    }
                    Ok(Ok(None)) => {
                        pty.close();
                        break None;
                    }
                    Ok(Err(_)) | Err(_) if tokio::time::Instant::now() < until => continue,
                    _ => {
                        pty.close();
                        break None;
                    }
                }
            };
            let Some((next, sid, pending)) = reattached else {
                exit = pty.wait().await?;
                break;
            };
            socket = Some(next);
            session_id = sid;
            queued = pending;
            last_reattach = Some(tokio::time::Instant::now());
            reconnects += 1;
            intentional = false;
            transport_fault = false;
        }
        let current = socket.as_mut().expect("connected wrapper");
        enum Event {
            Frame(io::Result<Box<Message>>),
            Output(Result<OutputChunk, broadcast::error::RecvError>),
            Ack(Option<InputResult>),
            Exit(io::Result<PtyExit>),
            Cancel,
        }
        let event = if let Some(frame) = queued.pop_front() {
            Event::Frame(Ok(Box::new(frame)))
        } else {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Event::Cancel,
                result = pty.wait() => Event::Exit(result),
                result = acks.recv() => Event::Ack(result),
                result = current.receive() => Event::Frame(result.map(Box::new)),
                result = output.recv(), if output_open => Event::Output(result),
            }
        };
        match event {
            Event::Cancel => {
                pty.close();
                exit = pty.wait().await?;
                break;
            }
            Event::Exit(result) => {
                exit = result?;
                break;
            }
            Event::Ack(Some(InputResult::Failed)) => {
                eprintln!("wrapper: PTY input write failed");
            }
            Event::Ack(Some(InputResult::Ack(seq))) => {
                let frame = Message {
                    r#type: "pty_input_ack".into(),
                    session_id,
                    input_seq: seq,
                    ..Default::default()
                };
                if send(current.as_mut(), &frame, options.write_timeout)
                    .await
                    .is_err()
                {
                    transport_fault = true;
                    socket.take();
                }
            }
            Event::Ack(None) => {
                pty.close();
                exit = pty.wait().await?;
                break;
            }
            Event::Frame(Err(_)) => {
                socket.take();
            }
            Event::Frame(Ok(frame)) => match frame.r#type.as_str() {
                "pty_input" if !frame.data.is_empty() => {
                    crate::logging::input_probe::wrapper_receive(
                        session_id,
                        frame.input_seq,
                        false,
                        &frame.data,
                    );
                    marks
                        .lock()
                        .map_err(|_| poison())?
                        .received(frame.input_seq);
                    if input_tx
                        .try_send(InputFrame {
                            seq: frame.input_seq,
                            data: frame.data,
                            attach: false,
                        })
                        .is_err()
                    {
                        // Retry remains possible: watermark was received, never processed.
                        transport_fault = true;
                        socket.take();
                    }
                }
                "attach_file" if !frame.inject.is_empty() => {
                    crate::logging::input_probe::wrapper_receive(
                        session_id,
                        0,
                        true,
                        frame.inject.as_bytes(),
                    );
                    if input_tx
                        .try_send(InputFrame {
                            seq: 0,
                            data: frame.inject.into_bytes(),
                            attach: true,
                        })
                        .is_err()
                    {
                        // Unsequenced attachment cannot safely be replayed.
                        pty.close();
                        exit = pty.wait().await?;
                        break;
                    }
                }
                "pty_resize" => {
                    if let Some(size) = PtySize::from_wire(frame.cols, frame.rows) {
                        // Source treats native resize as best effort, including
                        // races with provider exit. Keep unrelated input live.
                        let _ = pty.resize(size);
                    }
                }
                "hub_shutdown" => {
                    intentional = true;
                    socket.take();
                }
                "session_dismissed" => {
                    pty.close();
                    exit = pty.wait().await?;
                    break;
                }
                _ => {}
            },
            Event::Output(Ok(chunk)) => {
                if chunk.end <= sent_offset {
                    continue;
                }
                let start = chunk.end - chunk.bytes.len() as i64;
                let skip = sent_offset.saturating_sub(start).max(0) as usize;
                let frame = Message {
                    r#type: "pty_data".into(),
                    session_id,
                    data: chunk.bytes[skip..].to_vec(),
                    ..Default::default()
                };
                if send(current.as_mut(), &frame, options.write_timeout)
                    .await
                    .is_err()
                {
                    transport_fault = true;
                    socket.take();
                } else {
                    sent_offset = chunk.end;
                }
            }
            Event::Output(Err(broadcast::error::RecvError::Lagged(_))) => {
                transport_fault = true;
                socket.take();
            }
            Event::Output(Err(broadcast::error::RecvError::Closed)) => {
                output_open = false;
            }
        }
    }
    // Finish readers before final output/session_end; descendants holding PTY
    // fds never make wrapper completion unbounded.
    let _ = tokio::time::timeout(options.drain_timeout, async {
        while !*reader_done.borrow_and_update() {
            if reader_done.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    let (tail, total) = replay.lock().map_err(|_| poison())?.snapshot();
    if let Some(mut socket) = socket {
        let start = total - tail.len() as i64;
        let skip = sent_offset
            .saturating_sub(start)
            .max(0)
            .min(tail.len() as i64) as usize;
        for bytes in tail[skip..].chunks(256 * 1024) {
            let frame = Message {
                r#type: "pty_data".into(),
                session_id,
                data: bytes.to_vec(),
                ..Default::default()
            };
            if send(socket.as_mut(), &frame, options.write_timeout)
                .await
                .is_err()
            {
                break;
            }
        }
        let end = Message {
            r#type: "session_end".into(),
            session_id,
            state: exit.state().into(),
            exit_code: exit.code,
            signal: exit.signal.clone(),
            ..Default::default()
        };
        let _ = send(socket.as_mut(), &end, options.write_timeout).await;
    }
    Ok(WrapperResult {
        session_id,
        exit,
        reconnects,
        output_bytes: total,
    })
}
