use super::*;
use crate::hub::network::{self, Policy};
use std::sync::atomic::{AtomicBool, Ordering};
pub struct NativeOrchestrateIo {
    pub paths: RuntimePaths,
}
impl OrchestrateIo for NativeOrchestrateIo {
    fn exchange<'a>(
        &'a self,
        request: HttpRequest<'a>,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<HttpReply, RequestError>> {
        Box::pin(async move {
            if self.paths.is_trial() && request.port != self.paths.port() {
                return Err(RequestError::Transport);
            }
            let raw = format!("http://127.0.0.1:{}{}", request.port, request.path);
            let url = network::validate_url(&raw, Policy::ManagedLoopbackHttp)
                .map_err(|_| RequestError::Transport)?;
            if url.host_str() != Some("127.0.0.1") || !request.path.starts_with("/api/sessions/") {
                return Err(RequestError::Transport);
            }
            let headers_received = AtomicBool::new(false);
            let operation = async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(request.timeout)
                    .build()
                    .map_err(|_| RequestError::Transport)?;
                let builder = if let Some(body) = request.body {
                    client.post(url).json(&body)
                } else {
                    client.get(url)
                };
                let mut response =
                    builder
                        .bearer_auth(request.token)
                        .send()
                        .await
                        .map_err(|error| {
                            if error.is_timeout() {
                                RequestError::Timeout
                            } else {
                                RequestError::Transport
                            }
                        })?;
                headers_received.store(true, Ordering::Relaxed);
                let status = response.status().as_u16();
                let mut body = vec![];
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| RequestError::Transport)?
                {
                    if body.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
                        return Err(RequestError::Transport);
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(HttpReply { status, body })
            };
            tokio::select! {_=cancel.cancelled()=>Err(RequestError::Cancelled),result=tokio::time::timeout(request.timeout,operation)=>result.map_err(|_|if headers_received.load(Ordering::Relaxed){RequestError::Transport}else{RequestError::Timeout})?}
        })
    }
}
