//! Loopback-only wrapper transport. Tokens and ephemeral proofs deliberately do
//! not implement Debug and never enter error strings or serialized messages.
use crate::proto::{
    Message,
    core::{CoreFuture, SPAWN_PROOF_HEADER},
};
use futures_util::{SinkExt, StreamExt};
use std::{io, time::Duration};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message as WsMessage, client::IntoClientRequest},
};

pub trait HubSocket: Send {
    fn send<'a>(&'a mut self, frame: &'a Message) -> CoreFuture<'a, io::Result<()>>;
    fn receive(&mut self) -> CoreFuture<'_, io::Result<Message>>;
}
pub trait HubConnector: Send + Sync {
    fn connect<'a>(
        &'a self,
        proof: Option<&'a str>,
    ) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>>;
    fn probe(&self) -> CoreFuture<'_, bool>;
}
pub struct LoopbackConnector {
    port: u16,
    token: String,
    timeout: Duration,
}
impl LoopbackConnector {
    pub fn new(port: u16, token: String, timeout: Duration) -> io::Result<Self> {
        if port == 0 || timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "wrapper transport needs port and timeout",
            ));
        }
        Ok(Self {
            port,
            token,
            timeout,
        })
    }
}
struct Socket(WebSocketStream<TcpStream>);
fn connection_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "wrapper Hub transport failed",
    )
}
impl HubSocket for Socket {
    fn send<'a>(&'a mut self, frame: &'a Message) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            let text = serde_json::to_string(frame).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "wrapper frame cannot be encoded",
                )
            })?;
            self.0
                .send(WsMessage::Text(text.into()))
                .await
                .map_err(|_| connection_error())
        })
    }
    fn receive(&mut self) -> CoreFuture<'_, io::Result<Message>> {
        Box::pin(async move {
            loop {
                match self.0.next().await {
                    Some(Ok(WsMessage::Text(text))) => {
                        return crate::proto::decode_wire(text.as_bytes()).map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid wrapper Hub JSON frame",
                            )
                        });
                    }
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        return crate::proto::decode_wire(&bytes).map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid wrapper Hub JSON frame",
                            )
                        });
                    }
                    Some(Ok(WsMessage::Ping(_))) => {
                        self.0.flush().await.map_err(|_| connection_error())?
                    }
                    Some(Ok(WsMessage::Pong(_))) | Some(Ok(WsMessage::Frame(_))) => {}
                    _ => return Err(connection_error()),
                }
            }
        })
    }
}
impl HubConnector for LoopbackConnector {
    fn connect<'a>(
        &'a self,
        proof: Option<&'a str>,
    ) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>> {
        Box::pin(async move {
            let mut request = format!("ws://127.0.0.1:{}/ws", self.port)
                .into_client_request()
                .map_err(|_| connection_error())?;
            request.headers_mut().insert(
                "Origin",
                format!("http://127.0.0.1:{}", self.port)
                    .parse()
                    .map_err(|_| connection_error())?,
            );
            if let Some(proof) = proof {
                request.headers_mut().insert(
                    SPAWN_PROOF_HEADER,
                    proof.parse().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "invalid internal spawn proof")
                    })?,
                );
            }
            let socket = tokio::time::timeout(self.timeout, async {
                let stream = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.port)).await?;
                let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(4 * 1024 * 1024))
                    .max_frame_size(Some(4 * 1024 * 1024));
                let (socket, _) =
                    tokio_tungstenite::client_async_with_config(request, stream, Some(config))
                        .await
                        .map_err(|_| connection_error())?;
                Ok::<_, io::Error>(socket)
            })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "wrapper Hub dial timed out"))??;
            Ok(Box::new(Socket(socket)) as Box<dyn HubSocket>)
        })
    }
    fn probe(&self) -> CoreFuture<'_, bool> {
        Box::pin(async move {
            let Ok(client) = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(1))
                .build()
            else {
                return false;
            };
            client
                .get(format!("http://127.0.0.1:{}/", self.port))
                .query(&[("token", &self.token)])
                .send()
                .await
                .is_ok_and(|response| response.status() == reqwest::StatusCode::OK)
        })
    }
}

pub async fn register(
    connector: &dyn HubConnector,
    registration: &Message,
    proof: Option<&str>,
    timeout: Duration,
    cancel: &crate::process::Cancellation,
) -> io::Result<(Box<dyn HubSocket>, Message)> {
    let handshake = async {
        let mut socket = connector.connect(proof).await?;
        socket.send(registration).await?;
        let response = socket.receive().await?;
        if response.r#type != "registered" || response.session_id <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected wrapper registration response",
            ));
        }
        Ok((socket, response))
    };
    tokio::select! {
        _=cancel.cancelled()=>Err(io::Error::new(io::ErrorKind::Interrupted,"wrapper cancelled before registration")),
        result=tokio::time::timeout(timeout,handshake)=>result.map_err(|_|io::Error::new(io::ErrorKind::TimedOut,"wrapper registration timed out"))?,
    }
}
