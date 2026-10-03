//! WebSocket framing and callers. Core owns sessions, input FIFO, authorization
//! generations and UI replay/live ordering; the socket table owns only writers.
use super::{
    ServiceRouter, auth,
    http::{Request, Response},
    sockets::{FrameWriter, SocketRegistry, WireFrame},
};
use crate::{
    config::Config,
    process::Cancellation,
    proto::{self, core::*},
};
use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::{
    SinkExt, StreamExt,
    stream::{FuturesUnordered, SplitSink, SplitStream},
};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;
pub const UI_PING_INTERVAL: Duration = Duration::from_secs(30);
type Warning = dyn Fn(&str, &SessionError) + Send + Sync;

pub struct WebSocketService {
    services: Arc<ServiceRouter>,
    core: Arc<dyn SessionCore>,
    sockets: Arc<SocketRegistry>,
    effects: Arc<dyn CoreEffectSink>,
    shutdown: Cancellation,
    warning: Arc<Warning>,
}
impl WebSocketService {
    pub fn new(
        services: Arc<ServiceRouter>,
        core: Arc<dyn SessionCore>,
        sockets: Arc<SocketRegistry>,
        effects: Arc<dyn CoreEffectSink>,
        shutdown: Cancellation,
        warning: Arc<Warning>,
    ) -> Self {
        Self {
            services,
            core,
            sockets,
            effects,
            shutdown,
            warning,
        }
    }
    /// Called before upgrade. Unlike HTTP guards, WebSocket token authentication
    /// occurs only after receiving the first complete JSON frame.
    pub fn handshake(&self, request: &Request) -> Result<(), Response> {
        let snapshot = self
            .services
            .config
            .snapshot()
            .map_err(|_| Response::error(500, "internal", "configuration unavailable"))?;
        auth::websocket_origin(
            request,
            &snapshot.config,
            i64::from(self.services.bound_port()),
        )
    }
    pub async fn run(self: Arc<Self>, socket: WebSocket, request: Request) {
        if let Err(error) = self.run_inner(socket, request).await {
            (self.warning)("websocket closed", &error);
        }
    }
    async fn run_inner(&self, socket: WebSocket, request: Request) -> Result<(), SessionError> {
        let (writer, mut reader) = socket.split();
        let first = receive(&mut reader, &self.shutdown)
            .await?
            .ok_or(SessionError::Cancelled)?;
        // Capture before token checking; revoke between authentication and core
        // attach cannot install a UI in a newer authorization generation.
        let epoch = self.core.auth_epoch();
        let config = self
            .services
            .config
            .snapshot()
            .map_err(|_| SessionError::Shutdown)?;
        if !authorize_first(
            &request,
            &self.services.auth,
            &config.config,
            &first,
            SystemTime::now(),
        ) {
            return Err(SessionError::AuthenticationExpired);
        }
        let writer = Box::new(WsWriter(writer));
        if first.role == "ui" {
            let binding = UiBinding {
                connection: self.sockets.next_ui()?,
                auth_epoch: epoch,
            };
            let closed = self.sockets.insert_ui(binding, writer)?;
            let result = self.ui_loop(binding, first, &mut reader, &closed).await;
            // Exact binding only. A revoke may already have detached/closed it.
            let detached = self.core.detach_ui(binding);
            if let Err(failure) = self.effects.apply(detached).await {
                (self.warning)("UI detach effects", &failure.error);
            }
            self.sockets.close_ui(binding).await?;
            result
        } else {
            if !matches!(first.r#type.as_str(), "register" | "reattach") {
                return Err(SessionError::InvalidRequest(
                    "unknown WebSocket role".into(),
                ));
            }
            let spawn_proof = registration_proof(&request)?;
            if first.r#type == "reattach" && spawn_proof.is_some() {
                return Err(SessionError::InvalidRequest(
                    "registration proof is only valid for initial registration".into(),
                ));
            }
            let connection = self.sockets.next_wrapper()?;
            let closed = self.sockets.reserve_wrapper(connection, writer)?;
            let requested = first.session_id;
            let reattach = first.r#type == "reattach";
            let registration = if reattach {
                self.core
                    .reattach(
                        ReattachRequest { message: first },
                        connection,
                        SystemTime::now(),
                    )
                    .await
                    .map(|r| (r.binding, r.reattached, r.after_reattached))
            } else {
                SessionCore::register(
                    self.core.as_ref(),
                    RegisterRequest {
                        message: first,
                        spawn_proof,
                    },
                    connection,
                    SystemTime::now(),
                )
                .await
                .map(|r| (r.binding, r.registered, r.after_registered))
            };
            let (binding, ack, effects) = match registration {
                Ok(registration) => registration,
                Err(error) => {
                    if reattach && let Some(rejection) = reattach_rejection(requested, &error) {
                        let sent = self
                            .sockets
                            .reject_pending_wrapper(connection, rejection)
                            .await;
                        if let Err(error) = sent {
                            (self.warning)("reattach rejection", &error);
                        }
                    }
                    self.sockets.close_pending_wrapper(connection).await?;
                    return Err(error);
                }
            };
            if let Err(error) = self.sockets.bind_wrapper(binding) {
                self.sockets.close_pending_wrapper(connection).await?;
                if let Ok(effects) = self.core.disconnected(binding, SystemTime::now())
                    && let Err(failure) = self.apply(effects).await
                {
                    (self.warning)("failed registration cleanup", &failure);
                }
                return Err(error);
            }
            let result = async {
                self.sockets.acknowledge_wrapper(binding, ack).await?;
                self.apply(effects).await?;
                self.wrapper_loop(binding, &mut reader, &closed).await
            }
            .await;
            self.sockets.close_wrapper(binding).await?;
            match self.core.disconnected(binding, SystemTime::now()) {
                Ok(effects) => {
                    if let Err(error) = self.apply(effects).await {
                        (self.warning)("wrapper disconnect effects", &error);
                    }
                }
                Err(SessionError::StaleBinding | SessionError::NotFound(_)) => {}
                Err(error) => (self.warning)("wrapper disconnect", &error),
            }
            result
        }
    }
    async fn apply(&self, effects: CoreEffects) -> Result<(), SessionError> {
        self.effects
            .apply(effects)
            .await
            .map_err(|failure| failure.error)
    }
    async fn ui_loop(
        &self,
        binding: UiBinding,
        first: proto::Message,
        reader: &mut SplitStream<WebSocket>,
        closed: &Cancellation,
    ) -> Result<(), SessionError> {
        let active =
            (first.ui_active_session_id > 0).then_some(LiveSessionId(first.ui_active_session_id));
        let priming = self.core.attach_ui(binding, active, size(&first))?;
        if let Some(session) = active {
            match self
                .core
                .claim_ui_session(binding, session, size(&first), SystemTime::now())
            {
                Ok(effects) => self.apply(effects).await?,
                Err(SessionError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.sockets.send_ui_value(binding, &serde_json::json!({"type":"snapshot","hub_instance":priming.hub_instance,"sessions":priming.sessions})).await?;
        for frame in priming.ordered_frames {
            self.sockets.send_ui(binding, frame).await?;
        }
        loop {
            let batch = self.core.finish_ui_priming(binding)?;
            if batch.0.is_empty() {
                break;
            }
            self.apply(batch).await?;
        }
        let cancel = TaskCancellation::default();
        let ended = Cancellation::default();
        let (inputs, receiver) = tokio::sync::mpsc::unbounded_channel();
        let receive_loop = async {
            let result = loop {
                tokio::select! {
                    _ = self.shutdown.cancelled() => break Ok(()),
                    _ = closed.cancelled() => break Ok(()),
                    _ = ended.cancelled() => break Ok(()),
                incoming = receive(reader, &self.shutdown) => {
                    let message = match incoming { Ok(Some(message)) => message, Ok(None) => break Ok(()), Err(error) => break Err(error) };
                    if self.core.auth_epoch() != binding.auth_epoch { break Err(SessionError::AuthenticationExpired); }
                    let session = LiveSessionId(message.session_id);
                    let at = SystemTime::now();
                    let _accepted = if message.r#type == "pty_input" { None } else {
                        match self.core.authorize_ui_work(binding) {
                            Ok(permit) => Some(permit),
                            Err(error) => break Err(error),
                        }
                    };
                    let effects = match message.r#type.as_str() {
                        "pty_input" => {
                            match self.core.claim_ui_session(binding, session, None, at) {
                                Ok(effects) => if let Err(error) = self.apply(effects).await { break Err(error); },
                                Err(SessionError::NotFound(_)) => continue,
                                Err(error) => break Err(error),
                            }
                            if let Some(details) = self.core.details(session) {
                                // Invocation reserves the FIFO ticket synchronously;
                                // polling concurrent futures cannot reorder the frames.
                                if inputs.send(self.core.submit(details.binding, InputRequest { bytes: message.text.into_bytes(), authority: InputAuthority::Ui(binding) }, at, &cancel)).is_err() {
                                    break Err(SessionError::Shutdown);
                                }
                            }
                            continue;
                        }
                        "ui_active_session" => self.core.claim_ui_session(binding, session, size(&message), at),
                        "approval_consumed" => self.core.consume_approval(binding, message, at),
                        "approval_resync" => self.core.resync_approval(binding, session, at),
                        "pty_resize" => {
                            let (_, effects) = self.core.resize(binding, session, TerminalSize { cols: message.cols, rows: message.rows }, at);
                            Ok(effects)
                        }
                        "session_history_reset" => self.core.reset_history_from_ui(binding, session, at),
                        "session_dismiss" => self.core.dismiss_from_ui(binding, session, at),
                        "attach_request" => self.services.record_ws_attachment(&message).map(|_| CoreEffects::default()),
                        _ => continue,
                    };
                    match effects {
                        Ok(effects) => if let Err(error) = self.apply(effects).await { break Err(error); },
                        Err(SessionError::NotFound(_)) => {},
                        Err(SessionError::AuthenticationExpired) => break Err(SessionError::AuthenticationExpired),
                        Err(error) => (self.warning)("UI message", &error),
                    }
                }
                }
            };
            ended.cancel();
            // Detach membership first. The independent pump continues polling
            // accepted jobs while DrainUi waits, so their delayed Enter is not
            // vetoed by disconnect or a slow write to this browser.
            let detached = self.core.detach_ui(binding);
            if self.shutdown.is_cancelled() {
                cancel.cancel();
            }
            drop(inputs);
            if let Err(error) = self.apply(detached).await {
                (self.warning)("UI detach effects", &error);
            }
            result
        };
        let (result, _, _) = tokio::join!(
            receive_loop,
            pump_inputs(receiver, self.warning.as_ref()),
            self.ping_loop(binding, &ended),
        );
        result
    }
    async fn ping_loop(&self, binding: UiBinding, ended: &Cancellation) {
        let mut ping = tokio::time::interval_at(
            tokio::time::Instant::now() + UI_PING_INTERVAL,
            UI_PING_INTERVAL,
        );
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ended.cancelled() => return,
                _ = self.shutdown.cancelled() => return,
                _ = ping.tick() => {
                    let value = serde_json::json!({"type":"ping"});
                    let sent = tokio::select! {
                        _ = ended.cancelled() => return,
                        sent = self.sockets.send_ui_value(binding, &value) => sent,
                    };
                    if let Err(error) = sent {
                        (self.warning)("UI keepalive", &error);
                        ended.cancel();
                        return;
                    }
                }
            }
        }
    }
    async fn wrapper_loop(
        &self,
        binding: SessionBinding,
        reader: &mut SplitStream<WebSocket>,
        closed: &Cancellation,
    ) -> Result<(), SessionError> {
        loop {
            let message = tokio::select! {
                _ = closed.cancelled() => return Ok(()),
                incoming = receive(reader, &self.shutdown) => match incoming? { Some(message) => message, None => return Ok(()) },
            };
            let effects = match message.r#type.as_str() {
                "pty_data" => self.core.observe_output(
                    binding,
                    OutputChunk {
                        bytes: message.data,
                        total_pty_bytes: message.pty_bytes,
                    },
                    SystemTime::now(),
                )?,
                "pty_input_ack" => {
                    self.core.acknowledge(binding, InputSeq(message.input_seq));
                    continue;
                }
                "session_end" => self.core.observe_end(
                    binding,
                    SessionEnd {
                        declared_state: message.state,
                        exit_code: message.exit_code,
                        reason: message.reason,
                    },
                    SystemTime::now(),
                )?,
                _ => continue,
            };
            self.apply(effects).await?;
        }
    }
}
async fn pump_inputs(
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<CoreFuture<'_, InputReceipt>>,
    warning: &Warning,
) {
    let mut jobs = FuturesUnordered::new();
    let mut receiving = true;
    while receiving || !jobs.is_empty() {
        tokio::select! {
            input = receiver.recv(), if receiving => match input {
                Some(input) => jobs.push(input),
                None => receiving = false,
            },
            Some(receipt) = jobs.next(), if !jobs.is_empty() => {
                let receipt: InputReceipt = receipt;
                if matches!(receipt.disposition, InputDisposition::Failed { .. }) {
                    warning("UI input delivery", &SessionError::Transport("input delivery incomplete".into()));
                }
            }
        }
    }
}
fn registration_proof(request: &Request) -> Result<Option<String>, SessionError> {
    let mut values = request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(SPAWN_PROOF_HEADER));
    let Some((_, claim)) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some()
        || claim.len() != 64
        || !claim
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(SessionError::InvalidRequest(
            "invalid internal registration proof".into(),
        ));
    }
    Ok(Some(claim.clone()))
}
fn reattach_rejection(requested: i64, error: &SessionError) -> Option<proto::Message> {
    let SessionError::InvalidRequest(reason) = error else {
        return None;
    };
    if !matches!(
        reason.as_str(),
        "invalid session_id" | "session dismissed" | "invalid replay_b64"
    ) {
        return None;
    }
    Some(proto::Message {
        r#type: "reattach_reject".into(),
        session_id: if reason == "invalid session_id" {
            0
        } else {
            requested
        },
        reason: reason.clone(),
        ..Default::default()
    })
}
fn size(message: &proto::Message) -> Option<TerminalSize> {
    (message.cols > 0 && message.rows > 0).then_some(TerminalSize {
        cols: message.cols,
        rows: message.rows,
    })
}
fn authorize_first(
    request: &Request,
    auth_service: &super::pin::AuthService,
    config: &Config,
    first: &proto::Message,
    now: SystemTime,
) -> bool {
    let authenticated =
        auth::valid_token(&first.token, &config.token) || auth::token_or_trusted(request, config);
    let pin_required = auth::logically_remote(request) && !config.remote_pin_hash.trim().is_empty();
    authenticated
        && (!pin_required
            || auth_service.has_valid_pin(
                request,
                config,
                now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
            ))
}
struct WsWriter(SplitSink<WebSocket, WsMessage>);
impl FrameWriter for WsWriter {
    fn write<'a>(&'a mut self, frame: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let message = match frame {
                WireFrame::Text(text) => WsMessage::Text(text.into()),
                WireFrame::Close => WsMessage::Close(None),
            };
            self.0
                .send(message)
                .await
                .map_err(|error| SessionError::Transport(error.to_string()))
        })
    }
}
async fn receive(
    reader: &mut SplitStream<WebSocket>,
    cancel: &Cancellation,
) -> Result<Option<proto::Message>, SessionError> {
    loop {
        let frame = tokio::select! { _ = cancel.cancelled() => return Ok(None), frame = reader.next() => match frame { Some(Ok(frame)) => frame, Some(Err(error)) => return Err(SessionError::Transport(error.to_string())), None => return Ok(None) } };
        let bytes = match frame {
            WsMessage::Text(text) => text.as_bytes().to_vec(),
            WsMessage::Binary(bytes) => bytes.to_vec(),
            WsMessage::Close(_) => return Ok(None),
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
        };
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(SessionError::InvalidRequest(
                "WebSocket message exceeds limit".into(),
            ));
        }
        return decode_payload(&bytes).map(Some);
    }
}

fn decode_payload(bytes: &[u8]) -> Result<proto::Message, SessionError> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(SessionError::InvalidRequest(
            "WebSocket message exceeds limit".into(),
        ));
    }
    proto::decode_wire(bytes)
        .map_err(|_| SessionError::InvalidRequest("invalid WebSocket JSON".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::pin::AuthService;
    fn request() -> Request {
        Request {
            method: "GET".into(),
            path: "/ws".into(),
            host: "127.0.0.1:49200".into(),
            remote_addr: "127.0.0.1:54321".into(),
            ..Default::default()
        }
    }
    #[test]
    fn first_message_token_or_http_cookie_fallback() {
        let config = Config {
            token: "synthetic-hub-token".into(),
            ..Default::default()
        };
        let auth = AuthService::default();
        let mut request = request();
        let mut first = proto::Message {
            token: config.token.clone(),
            ..Default::default()
        };
        assert!(authorize_first(
            &request, &auth, &config, &first, UNIX_EPOCH
        ));
        first.token = "stale-local-storage-value".into();
        assert!(!authorize_first(
            &request, &auth, &config, &first, UNIX_EPOCH
        ));
        request.headers.push((
            "Cookie".into(),
            format!("{}={}", auth::TOKEN_COOKIE, config.token),
        ));
        assert!(authorize_first(
            &request, &auth, &config, &first, UNIX_EPOCH
        ));
    }
    #[test]
    fn remote_first_frame_also_requires_live_pin_cookie() {
        let config = Config {
            token: "synthetic-hub-token".into(),
            remote_pin_hash: "synthetic-nonempty-hash".into(),
            ..Default::default()
        };
        let auth = AuthService::default();
        let first = proto::Message {
            token: config.token.clone(),
            ..Default::default()
        };
        let mut request = request();
        assert!(authorize_first(
            &request, &auth, &config, &first, UNIX_EPOCH
        ));
        request
            .headers
            .push(("X-Forwarded-Proto".into(), "https".into()));
        assert!(!authorize_first(
            &request, &auth, &config, &first, UNIX_EPOCH
        ));
    }
    #[test]
    fn websocket_uses_whole_frame_decoder_and_go_byte_rules() {
        let frame =
            decode_payload(br#"{"type":"pty_data","data":[0,255],"unknown":1e999}"#).unwrap();
        assert_eq!(frame.data, [0, 255]);
        assert!(decode_payload(br#"{"type":"pty_data"} {}"#).is_err());
        assert!(decode_payload(br#"{"type":"pty_data","data":"invalid!"}"#).is_err());
        let bytes = vec![b' '; MAX_PAYLOAD_BYTES + 1];
        assert!(decode_payload(&bytes).is_err());
    }
    #[test]
    fn reattach_rejections_match_wire_reasons_without_internal_details() {
        let invalid = reattach_rejection(
            -1,
            &SessionError::InvalidRequest("invalid session_id".into()),
        )
        .unwrap();
        assert_eq!(invalid.r#type, "reattach_reject");
        assert_eq!(invalid.session_id, 0);
        let dismissed =
            reattach_rejection(7, &SessionError::InvalidRequest("session dismissed".into()))
                .unwrap();
        assert_eq!(dismissed.session_id, 7);
        assert!(
            reattach_rejection(7, &SessionError::Transport("private diagnostic".into())).is_none()
        );
    }
    #[test]
    fn internal_registration_header_is_bounded_one_claim_and_never_disclosed() {
        let mut request = request();
        assert!(registration_proof(&request).unwrap().is_none());
        let proof = "a".repeat(64);
        request
            .headers
            .push((SPAWN_PROOF_HEADER.into(), proof.clone()));
        assert_eq!(registration_proof(&request).unwrap(), Some(proof.clone()));
        request
            .headers
            .push((SPAWN_PROOF_HEADER.to_uppercase(), proof));
        assert!(registration_proof(&request).is_err());
        for invalid in [
            String::new(),
            "b".repeat(65),
            "A".repeat(64),
            "z".repeat(64),
        ] {
            request.headers.truncate(1);
            request.headers[0].1 = invalid.clone();
            let error = registration_proof(&request).unwrap_err();
            assert_eq!(
                error,
                SessionError::InvalidRequest("invalid internal registration proof".into())
            );
        }
    }
}

#[cfg(test)]
#[path = "ws_transport_tests.rs"]
mod transport_tests;
