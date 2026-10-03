//! Connection ownership and ordered effect delivery. Session state and UI
//! priming remain exclusively in SessionCore; this table stores socket handles.
use crate::{
    process::Cancellation,
    proto::{self, core::*},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, watch};

pub const UI_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
pub const STOP_WRITE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireFrame {
    Text(String),
    Close,
}
/// The production adapter owns the actual WebSocket split sink. Tests inject
/// only synthetic writers; a successful call means the frame was flushed.
pub trait FrameWriter: Send {
    fn write<'a>(&'a mut self, frame: WireFrame) -> CoreFuture<'a, Result<(), SessionError>>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    Ui(UiBinding),
    Wrapper(SessionBinding),
}
type Authority = dyn Fn(Binding) -> bool + Send + Sync;
type WarningHandler = Arc<dyn Fn(&str, &SessionError) + Send + Sync>;
struct Socket {
    binding: OnceLock<Binding>,
    writer: AsyncMutex<Box<dyn FrameWriter>>,
    ready: watch::Sender<bool>,
    closed: Cancellation,
}
impl Socket {
    fn new(binding: Option<Binding>, writer: Box<dyn FrameWriter>, ready: bool) -> Self {
        Self {
            binding: binding.map(OnceLock::from).unwrap_or_default(),
            writer: AsyncMutex::new(writer),
            ready: watch::channel(ready).0,
            closed: Cancellation::default(),
        }
    }
    async fn wait_ready(&self) -> Result<(), SessionError> {
        let mut ready = self.ready.subscribe();
        loop {
            if self.closed.is_cancelled() {
                return Err(SessionError::Transport("socket closed".into()));
            }
            if *ready.borrow_and_update() {
                return Ok(());
            }
            tokio::select! {
                _ = self.closed.cancelled() => return Err(SessionError::Transport("socket closed".into())),
                result = ready.changed() => if result.is_err() { return Err(SessionError::Shutdown); },
            }
        }
    }
}
#[derive(Default)]
struct Connections {
    wrappers: BTreeMap<WrapperConnectionId, Arc<Socket>>,
    uis: BTreeMap<UiConnectionId, Arc<Socket>>,
}
#[derive(Default)]
pub struct SocketRegistry {
    connections: Mutex<Connections>,
    next_connection: AtomicU64,
    authority: OnceLock<Arc<Authority>>,
    core: OnceLock<Weak<dyn SessionCore>>,
}
impl SocketRegistry {
    /// Break the constructor cycle once: construct registry and effect driver,
    /// construct core using them, then bind that exact core before accepting.
    pub fn bind_core(&self, core: Weak<dyn SessionCore>) -> Result<(), SessionError> {
        self.core.set(core.clone()).map_err(|_| {
            SessionError::InvalidRequest("socket registry core already bound".into())
        })?;
        self.authority
            .set(Arc::new(move |binding| {
                core.upgrade().is_some_and(|core| match binding {
                    Binding::Wrapper(binding) => core.is_current(binding),
                    Binding::Ui(binding) => core.auth_epoch() == binding.auth_epoch,
                })
            }))
            .map_err(|_| SessionError::InvalidRequest("socket registry core already bound".into()))
    }
    fn authorized(&self, binding: Binding) -> bool {
        self.authority.get().is_some_and(|check| check(binding))
    }
    fn next_id(&self) -> Result<u64, SessionError> {
        self.next_connection
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map(|n| n + 1)
            .map_err(|_| SessionError::InvalidRequest("socket connection IDs exhausted".into()))
    }
    pub fn next_wrapper(&self) -> Result<WrapperConnectionId, SessionError> {
        self.next_id().map(WrapperConnectionId)
    }
    pub fn next_ui(&self) -> Result<UiConnectionId, SessionError> {
        self.next_id().map(UiConnectionId)
    }
    /// Reserve the actual writer before core publishes its session binding.
    /// A concurrent resize/input waits for bind+ACK instead of seeing no writer.
    pub fn reserve_wrapper(
        &self,
        connection: WrapperConnectionId,
        writer: Box<dyn FrameWriter>,
    ) -> Result<Cancellation, SessionError> {
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| SessionError::Shutdown)?;
        if connections.wrappers.contains_key(&connection) {
            return Err(SessionError::InvalidRequest(
                "duplicate wrapper connection".into(),
            ));
        }
        let socket = Arc::new(Socket::new(None, writer, false));
        let cancel = socket.closed.clone();
        connections.wrappers.insert(connection, socket);
        Ok(cancel)
    }
    pub fn bind_wrapper(&self, binding: SessionBinding) -> Result<(), SessionError> {
        let socket = self.socket(Binding::Wrapper(binding))?;
        socket
            .binding
            .set(Binding::Wrapper(binding))
            .map_err(|_| SessionError::InvalidRequest("wrapper identity already bound".into()))
    }
    pub fn insert_wrapper(
        &self,
        binding: SessionBinding,
        writer: Box<dyn FrameWriter>,
    ) -> Result<Cancellation, SessionError> {
        let cancel = self.reserve_wrapper(binding.wrapper, writer)?;
        self.bind_wrapper(binding)?;
        Ok(cancel)
    }
    /// Only an authenticated, still-unbound handshake can use this final reply.
    pub async fn reject_pending_wrapper(
        &self,
        connection: WrapperConnectionId,
        rejection: proto::Message,
    ) -> Result<(), SessionError> {
        if rejection.r#type != "reattach_reject" {
            return Err(SessionError::InvalidRequest(
                "invalid handshake rejection".into(),
            ));
        }
        let socket = self
            .connections
            .lock()
            .map_err(|_| SessionError::Shutdown)?
            .wrappers
            .get(&connection)
            .filter(|s| s.binding.get().is_none())
            .cloned()
            .ok_or(SessionError::StaleBinding)?;
        let mut writer = socket.writer.lock().await;
        tokio::select! { _ = socket.closed.cancelled() => Err(SessionError::Cancelled), result = writer.write(encode(&rejection)?) => result }
    }
    pub async fn close_pending_wrapper(
        &self,
        connection: WrapperConnectionId,
    ) -> Result<(), SessionError> {
        let socket = {
            let mut connections = self
                .connections
                .lock()
                .map_err(|_| SessionError::Shutdown)?;
            if connections
                .wrappers
                .get(&connection)
                .is_some_and(|s| s.binding.get().is_none())
            {
                connections.wrappers.remove(&connection)
            } else {
                None
            }
        };
        if let Some(socket) = socket {
            Self::close_socket(socket).await;
        }
        Ok(())
    }
    pub fn insert_ui(
        &self,
        binding: UiBinding,
        writer: Box<dyn FrameWriter>,
    ) -> Result<Cancellation, SessionError> {
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| SessionError::Shutdown)?;
        if connections.uis.contains_key(&binding.connection) {
            return Err(SessionError::InvalidRequest(
                "duplicate UI connection".into(),
            ));
        }
        let socket = Arc::new(Socket::new(Some(Binding::Ui(binding)), writer, true));
        let cancel = socket.closed.clone();
        connections.uis.insert(binding.connection, socket);
        Ok(cancel)
    }
    fn socket(&self, binding: Binding) -> Result<Arc<Socket>, SessionError> {
        let connections = self
            .connections
            .lock()
            .map_err(|_| SessionError::Shutdown)?;
        let socket = match binding {
            Binding::Ui(binding) => connections.uis.get(&binding.connection),
            Binding::Wrapper(binding) => connections.wrappers.get(&binding.wrapper),
        }
        .filter(|socket| socket.binding.get().is_none_or(|stored| *stored == binding));
        socket.cloned().ok_or(SessionError::StaleBinding)
    }
    fn remove(&self, binding: Binding) -> Result<Option<Arc<Socket>>, SessionError> {
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| SessionError::Shutdown)?;
        let socket =
            match binding {
                Binding::Ui(b)
                    if connections.uis.get(&b.connection).is_some_and(|s| {
                        s.binding.get().is_none_or(|stored| *stored == binding)
                    }) =>
                {
                    connections.uis.remove(&b.connection)
                }
                Binding::Wrapper(b)
                    if connections.wrappers.get(&b.wrapper).is_some_and(|s| {
                        s.binding.get().is_none_or(|stored| *stored == binding)
                    }) =>
                {
                    connections.wrappers.remove(&b.wrapper)
                }
                _ => None,
            };
        Ok(socket)
    }
    /// Acknowledgement bypasses only the registration gate. Other sends cannot
    /// overtake it, including input submitted concurrently with registration.
    pub async fn acknowledge_wrapper(
        &self,
        binding: SessionBinding,
        message: proto::Message,
    ) -> Result<(), SessionError> {
        if !matches!(message.r#type.as_str(), "registered" | "reattach_ack") {
            return Err(SessionError::InvalidRequest(
                "invalid wrapper acknowledgement".into(),
            ));
        }
        let socket = self.socket(Binding::Wrapper(binding))?;
        let mut writer = socket.writer.lock().await;
        if *socket.ready.borrow() {
            return Err(SessionError::InvalidRequest(
                "wrapper already acknowledged".into(),
            ));
        }
        if socket.binding.get() != Some(&Binding::Wrapper(binding))
            || !self.authorized(Binding::Wrapper(binding))
        {
            return Err(SessionError::StaleBinding);
        }
        let frame = encode(&message)?;
        tokio::select! {
            _ = socket.closed.cancelled() => return Err(SessionError::Cancelled),
            result = writer.write(frame) => result?,
        }
        socket.ready.send_replace(true);
        Ok(())
    }
    async fn send_frame(
        &self,
        binding: Binding,
        frame: WireFrame,
        timeout: Option<Duration>,
        check_authority: bool,
    ) -> Result<(), SessionError> {
        let socket = self.socket(binding)?;
        let send = async {
            socket.wait_ready().await?;
            let mut writer = socket.writer.lock().await;
            // Recheck after awaiting the writer lock; reattach/revoke may have
            // invalidated the request while a preceding write was in flight.
            if socket.binding.get() != Some(&binding)
                || (check_authority && !self.authorized(binding))
            {
                return Err(match binding {
                    Binding::Ui(_) => SessionError::AuthenticationExpired,
                    _ => SessionError::StaleBinding,
                });
            }
            tokio::select! {
                _ = socket.closed.cancelled() => Err(SessionError::Cancelled),
                result = writer.write(frame) => result,
            }
        };
        let result = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, send)
                .await
                .unwrap_or(Err(SessionError::TimedOut)),
            None => send.await,
        };
        if matches!(
            result,
            Err(SessionError::Transport(_) | SessionError::TimedOut)
        ) {
            socket.closed.cancel();
        }
        result
    }
    pub async fn send_ui_value<T: Serialize + Sync>(
        &self,
        binding: UiBinding,
        value: &T,
    ) -> Result<(), SessionError> {
        self.send_frame(
            Binding::Ui(binding),
            encode(value)?,
            Some(UI_WRITE_TIMEOUT),
            true,
        )
        .await
    }
    pub async fn send_ui(
        &self,
        binding: UiBinding,
        message: proto::Message,
    ) -> Result<(), SessionError> {
        self.send_ui_value(binding, &message).await
    }
    async fn close_socket(socket: Arc<Socket>) {
        socket.closed.cancel();
        // Cancellation releases a blocked split-sink operation. Close remains
        // bounded even when the peer does not read its final control frame.
        let _ = tokio::time::timeout(STOP_WRITE_TIMEOUT, async {
            socket.writer.lock().await.write(WireFrame::Close).await
        })
        .await;
    }
    async fn close(&self, binding: Binding) -> Result<(), SessionError> {
        if let Some(socket) = self.remove(binding)? {
            Self::close_socket(socket).await;
        }
        Ok(())
    }
    pub async fn close_ui(&self, binding: UiBinding) -> Result<(), SessionError> {
        self.close(Binding::Ui(binding)).await
    }
    /// Deliberately does not consult core.is_current: reattach closes the old
    /// exact connection while the new incarnation already owns the session.
    pub async fn close_wrapper(&self, binding: SessionBinding) -> Result<(), SessionError> {
        self.close(Binding::Wrapper(binding)).await
    }
    async fn cancel_session(
        &self,
        binding: SessionBinding,
        reason: StopReason,
    ) -> Result<(), SessionError> {
        let reason = stop_reason(reason)?;
        let message = proto::Message {
            r#type: proto::TYPE_SESSION_DISMISSED.into(),
            session_id: binding.session.0,
            reason: reason.into(),
            ..Default::default()
        };
        // Core may already have removed the session. The exact socket binding
        // remains the authority for this final intent frame and close operation.
        let sent = self
            .send_frame(
                Binding::Wrapper(binding),
                encode(&message)?,
                Some(STOP_WRITE_TIMEOUT),
                false,
            )
            .await;
        self.close_wrapper(binding).await?;
        // A missing socket already satisfies cancellation; delivery errors are
        // reported without retrying or leaving the connection alive.
        match sent {
            Err(SessionError::StaleBinding) => Ok(()),
            result => result,
        }
    }
    pub async fn notify_hub_shutdown(&self, reason: &str) -> Result<(), SessionError> {
        let bindings: Vec<_> = self
            .connections
            .lock()
            .map_err(|_| SessionError::Shutdown)?
            .wrappers
            .values()
            .filter_map(|s| match s.binding.get() {
                Some(Binding::Wrapper(b)) => Some(*b),
                _ => None,
            })
            .collect();
        let message = proto::Message {
            r#type: "hub_shutdown".into(),
            reason: reason.into(),
            ..Default::default()
        };
        let mut first_error = None;
        for binding in bindings {
            if let Err(error) = self
                .send_frame(
                    Binding::Wrapper(binding),
                    encode(&message)?,
                    Some(STOP_WRITE_TIMEOUT),
                    false,
                )
                .await
            {
                first_error.get_or_insert(error);
            }
        }
        // This notification must never emit session_dismissed or stop the PTY.
        first_error.map_or(Ok(()), Err)
    }
}
impl WrapperTransport for SocketRegistry {
    fn send<'a>(
        &'a self,
        binding: SessionBinding,
        message: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.send_frame(Binding::Wrapper(binding), encode(&message)?, None, true)
                .await
        })
    }
}
fn encode<T: Serialize>(value: &T) -> Result<WireFrame, SessionError> {
    let bytes = proto::provider::to_go_json(value)
        .map_err(|error| SessionError::Transport(error.to_string()))?;
    String::from_utf8(bytes)
        .map(WireFrame::Text)
        .map_err(|error| SessionError::Transport(error.to_string()))
}
fn stop_reason(reason: StopReason) -> Result<&'static str, SessionError> {
    match reason {
        StopReason::KillAll => Ok("kill_all"),
        StopReason::IdleTimeout => Ok("idle_timeout"),
        StopReason::Dismissed => Ok("ui_dismiss"),
        StopReason::ParentEnded => Ok("parent_ended"),
        StopReason::StartupFailed => Ok("startup_failed"),
        StopReason::Timeout => Ok("timeout"),
        StopReason::User => Err(SessionError::InvalidRequest(
            "user stop needs a source-specific reason".into(),
        )),
        StopReason::HubShutdown => Err(SessionError::InvalidRequest(
            "Hub shutdown must preserve wrapper PTYs".into(),
        )),
    }
}
/// Observers whose completion matters to input ordering (notably Git turn
/// capture) run before the same event is published for asynchronous consumers.
pub trait OrderedEventObserver: Send + Sync {
    fn observe<'a>(&'a self, event: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>>;
}
pub struct EffectDriver {
    sockets: Arc<SocketRegistry>,
    persistence: Arc<dyn PersistenceEffectSink>,
    publisher: Arc<dyn CoreEventPublisher>,
    observer: Arc<dyn OrderedEventObserver>,
    warning: WarningHandler,
}
impl EffectDriver {
    pub fn new(
        sockets: Arc<SocketRegistry>,
        persistence: Arc<dyn PersistenceEffectSink>,
        publisher: Arc<dyn CoreEventPublisher>,
        observer: Arc<dyn OrderedEventObserver>,
    ) -> Self {
        Self {
            sockets,
            persistence,
            publisher,
            observer,
            warning: Arc::new(|operation, error| eprintln!("Hub effects {operation}: {error:?}")),
        }
    }
    pub fn with_warning_handler(mut self, warning: WarningHandler) -> Self {
        self.warning = warning;
        self
    }
    async fn apply_one(&self, effect: CoreEffect) -> Result<(), SessionError> {
        match effect {
            CoreEffect::SendWrapper { binding, message } => {
                self.sockets.send(binding, message).await
            }
            CoreEffect::SendUi { binding, message } => self.sockets.send_ui(binding, message).await,
            CoreEffect::SendUiBestEffort { binding, message } => {
                if let Err(error) = self.sockets.send_ui(binding, message).await {
                    (self.warning)("broadcast UI delivery", &error);
                    self.sockets.close_ui(binding).await?;
                }
                Ok(())
            }
            CoreEffect::CloseWrapper { binding } => self.sockets.close_wrapper(binding).await,
            CoreEffect::DrainUi(binding) => {
                let core = self
                    .sockets
                    .core
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or(SessionError::Shutdown)?;
                core.drain_ui_work(binding).await;
                Ok(())
            }
            CoreEffect::SendUiGitTurn {
                binding,
                event,
                best_effort,
            } => {
                let result = self.sockets.send_ui_value(binding, &event).await;
                if best_effort {
                    if let Err(error) = result {
                        (self.warning)("Git turn UI delivery", &error);
                        self.sockets.close_ui(binding).await?;
                    }
                    Ok(())
                } else {
                    result
                }
            }
            CoreEffect::Broadcast(_) | CoreEffect::BroadcastGitTurn(_) => {
                Err(SessionError::InvalidRequest(
                    "broadcast must pass through SessionCore UI priming".into(),
                ))
            }
            CoreEffect::Persist(effect) => self.persistence.apply(effect),
            CoreEffect::PersistBound {
                binding,
                scope,
                effect,
                order,
            } => {
                order.wait().await;
                let result = self.persistence.apply_bound(binding, scope, effect);
                drop(order);
                result
            }
            CoreEffect::Notify(event) => {
                self.observer.observe(&event).await?;
                self.publisher.publish(event).map(|_| ())
            }
            CoreEffect::CancelSession { binding, reason } => {
                self.sockets.cancel_session(binding, reason).await
            }
            CoreEffect::CloseUi(binding) => self.sockets.close_ui(binding).await,
        }
    }
}
impl CoreEffectSink for EffectDriver {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for (index, effect) in effects.0.into_iter().enumerate() {
                self.apply_one(effect)
                    .await
                    .map_err(|error| CoreEffectFailure { index, error })?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
