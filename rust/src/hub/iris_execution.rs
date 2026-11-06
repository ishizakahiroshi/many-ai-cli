//! Opt-in, separately authenticated execution contract. Admission is durable;
//! transport completion and session exit are never execution evidence.
//!
//! The production configuration deliberately has no launch backend until an
//! owner-written process/result receipt can satisfy the declared capabilities.
//! Synthetic backends are injected by isolated acceptance harnesses only.
use super::{
    http::{Request, Response},
    iris_execution_journal::{Acceptance, Binding, Journal},
    iris_execution_mcp as mcp,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub const PATH: &str = "/api/iris-execution/mcp";
type ToolResult = Result<Value, (String, String)>;

/// The backend is trusted application code, never a caller-supplied MCP tool.
/// Its events still undergo journal binding, sequencing and proof validation.
pub trait ExecutionBackend: Send + Sync {
    fn capabilities(&self) -> Value;
    fn start(&self, binding: &Binding) -> ToolResult;
    fn poll(&self, binding: &Binding) -> ToolResult;
    fn stop(&self, binding: &Binding, stop_request_id: &str) -> ToolResult;
}

#[derive(Clone)]
pub struct IrisExecutionHttp {
    token: String,
    principal: String,
    workspace: String,
    host: String,
    instance: String,
    port: u16,
    journal: Arc<Mutex<Journal>>,
    backend: Option<Arc<dyn ExecutionBackend>>,
    owner_store: Option<Arc<Mutex<super::iris_execution_owner::OwnerStore>>>,
    owned_profile: Option<Arc<super::iris_execution_owned::OwnedProfile>>,
    operator_grants_loaded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalIdentity {
    principal_id: String,
    workspace_id: String,
    execution_host_id: String,
}

impl IrisExecutionHttp {
    /// No files are created unless the operator explicitly supplies identity
    /// and a separate 256-bit credential under the selected runtime root.
    /// Grants cannot be issued through MCP. This initial production owner is
    /// inspection-only and advertises the missing live proof capabilities.
    pub fn from_local(root: &Path, port: u16) -> Result<Option<Self>, String> {
        let dir = root.join("iris-execution");
        let identity_path = dir.join("identity.json");
        if !identity_path.exists() {
            return Ok(None);
        }
        // Refuse a symlink, junction or hard link like the other Iris stores,
        // so a planted alias cannot substitute another file's contents.
        super::iris_execution_owner::check_store_path(&identity_path)
            .map_err(|_| "Execution identity must be a regular file")?;
        let bytes = std::fs::read(&identity_path).map_err(|_| "Execution identity unavailable")?;
        let value = mcp::parse_json(&bytes).map_err(|_| "Invalid execution identity")?;
        let identity: LocalIdentity =
            serde_json::from_value(value).map_err(|_| "Invalid execution identity fields")?;
        let credential_path = dir.join("credential");
        super::iris_execution_owner::check_store_path(&credential_path)
            .map_err(|_| "Separate execution credential must be a regular file")?;
        // Editors commonly add a trailing newline; it is never part of the value.
        let token = std::fs::read_to_string(&credential_path)
            .map_err(|_| "Separate execution credential unavailable")?
            .trim()
            .to_owned();
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| "Execution identity random source unavailable")?;
        let instance: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let mut journal = Journal::open(
            &dir.join("journal.sqlite3"),
            &identity.execution_host_id,
            &instance,
        )
        .map_err(|e| e.code)?;
        let mut operator_grants_loaded = false;
        if root.join("iris-execution-approved-grants.json").exists() {
            let mut loader =
                super::iris_execution_grants::GrantLoader::open(root).map_err(|e| e.code)?;
            for approved in loader.load_approved().map_err(|e| e.code)? {
                journal.register_grant(approved.grant).map_err(|e| e.code)?;
            }
            operator_grants_loaded = true;
        }
        let owner_store = super::iris_execution_owner::OwnerStore::open(&dir.join("owner.sqlite3"))
            .map_err(|e| e.code)?;
        let mut owner = Self::new(
            token,
            identity.principal_id,
            identity.workspace_id,
            identity.execution_host_id,
            instance,
            port,
            journal,
            None,
        )?;
        owner.owner_store = Some(Arc::new(Mutex::new(owner_store)));
        owner.operator_grants_loaded = operator_grants_loaded;
        Ok(Some(owner))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        token: String,
        principal: String,
        workspace: String,
        host: String,
        instance: String,
        port: u16,
        journal: Journal,
        backend: Option<Arc<dyn ExecutionBackend>>,
    ) -> Result<Self, String> {
        Self::new_shared(
            token,
            principal,
            workspace,
            host,
            instance,
            port,
            Arc::new(Mutex::new(journal)),
            backend,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_shared(
        token: String,
        principal: String,
        workspace: String,
        host: String,
        instance: String,
        port: u16,
        journal: Arc<Mutex<Journal>>,
        backend: Option<Arc<dyn ExecutionBackend>>,
    ) -> Result<Self, String> {
        if token.len() != 64
            || !token.bytes().all(|b| b.is_ascii_hexdigit())
            || port == 0
            || [&principal, &workspace, &host, &instance]
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
        {
            return Err("Invalid execution endpoint configuration".into());
        }
        Ok(Self {
            token,
            principal,
            workspace,
            host,
            instance,
            port,
            journal,
            backend,
            owner_store: None,
            owned_profile: None,
            operator_grants_loaded: false,
        })
    }

    pub fn preflight(&self, request: &Request) -> Result<(), Response> {
        let reject = |status, code, detail| Response::error(status, code, detail).no_store();
        // Never accept query/cookie/trusted-network authentication here.
        let auth: Vec<_> = request
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .collect();
        if auth.len() != 1
            || !auth[0]
                .1
                .strip_prefix("Bearer ")
                .is_some_and(|v| super::auth::valid_token(v, &self.token))
        {
            return Err(reject(
                401,
                "unauthorized",
                "Separate execution credential required",
            ));
        }
        let expected = format!("127.0.0.1:{}", self.port);
        let origins: Vec<_> = request
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("origin"))
            .collect();
        if request.host != expected
            || request.tls
            || !request.query.is_empty()
            || origins.len() > 1
            || origins
                .first()
                .is_some_and(|(_, v)| *v != format!("http://{expected}"))
            || !request
                .remote_addr
                .parse::<std::net::SocketAddr>()
                .is_ok_and(|a| a.ip().is_loopback())
        {
            return Err(reject(
                403,
                "execution_origin",
                "Local execution origin required",
            ));
        }
        if request.path != PATH || request.method != "POST" {
            return Err(reject(405, "method_not_allowed", "POST required"));
        }
        if request.body.len() > mcp::MAX_BYTES {
            return Err(reject(413, "body_too_large", "Execution body limit"));
        }
        Ok(())
    }

    pub fn handle_authenticated(&self, request: &Request, now: i64) -> Response {
        if let Err(error) = self.preflight(request) {
            return error;
        }
        let request_value = match mcp::parse_json(&request.body) {
            Ok(value) => value,
            Err(error) => {
                return Response::json(400, &mcp::error_response(None, &error)).no_store();
            }
        };
        let headers: Vec<_> = request
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if let Err(error) = mcp::validate_http(&headers, &request_value) {
            return Response::json(400, &mcp::error_response(request_value.get("id"), &error))
                .no_store();
        }
        let value = mcp::handle_with_tools(
            &request_value,
            "many-ai-cli-execution",
            &self.tools(),
            &mut |tool, args| self.call(tool, args, now),
        );
        if value.is_null() {
            return Response::bytes(202, "application/json", Vec::new()).no_store();
        }
        Response::json(200, &value).no_store()
    }

    pub fn capabilities(&self) -> Value {
        let mut value = self.legacy_capabilities();
        if let Some(profile) = &self.owned_profile {
            value["owned_dispatch"] = profile.capabilities();
        }
        value
    }

    fn legacy_capabilities(&self) -> Value {
        let backend = self
            .backend
            .as_ref()
            .map(|b| b.capabilities())
            .unwrap_or_else(|| json!({}));
        let synthetic = backend["synthetic"] == true && backend["external_effects"] == false;
        let ready =
            synthetic && backend["fenced_stop_proof"] == true && backend["result_sha256"] == true;
        json!({"contract_version":"iris-execution/1", "execution_host_id":self.host, "executor_instance_id":self.instance,
            "synthetic":synthetic, "external_effects":!synthetic, "durable_start_receipt":true,"query_by_attempt":true,
            "query_by_request":true,"grant_revision_binding":true,
            "fenced_stop_proof":synthetic && backend["fenced_stop_proof"]==true,
            "result_sha256":synthetic && backend["result_sha256"]==true,
            "execution_ready":ready,
            "live_provider_execution":false, "automatic_handoff":false,
            "owner_receipt_query":self.owner_store.is_some(),"operator_grants_loaded":self.operator_grants_loaded})
    }

    pub fn with_owner_store(mut self, store: super::iris_execution_owner::OwnerStore) -> Self {
        self.owner_store = Some(Arc::new(Mutex::new(store)));
        self
    }

    pub fn with_shared_owner_store(
        mut self,
        store: Arc<Mutex<super::iris_execution_owner::OwnerStore>>,
    ) -> Self {
        self.owner_store = Some(store);
        self
    }

    /// Trusted composition only. The HTTP owner and dispatcher must share the
    /// exact store objects, rather than opening another connection or journal.
    pub fn with_owned_profile(
        mut self,
        profile: Arc<super::iris_execution_owned::OwnedProfile>,
    ) -> Result<Self, String> {
        let owners = self
            .owner_store
            .as_ref()
            .ok_or("Owned profile requires owner store")?;
        if !profile.shares(&self.journal, owners) {
            return Err("Owned profile store identity mismatch".into());
        }
        self.owned_profile = Some(profile);
        Ok(self)
    }

    fn tools(&self) -> Vec<Value> {
        let mut definitions = tools();
        if self.owned_profile.is_some() {
            for name in [
                "owned_start",
                "owned_poll",
                "owned_stop",
                "owned_request",
                "owned_evidence",
            ] {
                let mut props = json!({"profile":{"type":"string"}});
                let mut required = vec!["profile"];
                if name != "owned_request" {
                    props["binding"] = json!({"type":"object"});
                    required.push("binding");
                }
                if ["owned_start", "owned_stop", "owned_request"].contains(&name) {
                    props["request_id"] = json!({"type":"string"});
                    required.push("request_id");
                }
                definitions.push(mcp::tool(name, props, &required));
            }
        }
        definitions
    }

    /// The async path preserves the same preflight, bounded parser and MCP
    /// envelope validation. The protocol layer validates arguments before any
    /// owned operation, and no journal guard crosses an await.
    pub async fn handle_authenticated_async(&self, request: &Request, now: i64) -> Response {
        if let Err(error) = self.preflight(request) {
            return error;
        }
        let input = match mcp::parse_json(&request.body) {
            Ok(value) => value,
            Err(error) => {
                return Response::json(400, &mcp::error_response(None, &error)).no_store();
            }
        };
        let headers: Vec<_> = request
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if let Err(error) = mcp::validate_http(&headers, &input) {
            return Response::json(400, &mcp::error_response(input.get("id"), &error)).no_store();
        }
        let definitions = self.tools();
        let owner = self.clone();
        let decoded = input.clone();
        let schema = definitions.clone();
        // Capabilities and legacy inspection also touch synchronous stores.
        // Keep their mutex/file waits off the runtime's stop/deadline lane.
        let prepared = tokio::task::spawn_blocking(move || {
            let mut pending = None;
            let preliminary = mcp::handle_with_tools(
                &decoded,
                "many-ai-cli-execution",
                &schema,
                &mut |tool, args| {
                    if tool.starts_with("owned_") {
                        pending = Some((tool.to_owned(), args.clone()));
                        Ok(Value::Null)
                    } else {
                        owner.call(tool, args, now)
                    }
                },
            );
            (preliminary, pending)
        })
        .await;
        let (preliminary, pending) = match prepared {
            Ok(result) => result,
            Err(_) => {
                return Response::error(
                    503,
                    "execution_unavailable",
                    "Execution inspection unavailable",
                )
                .no_store();
            }
        };
        let output = if let Some((tool, args)) = pending {
            let result = match &self.owned_profile {
                Some(profile) => {
                    profile
                        .call(
                            &tool,
                            &args,
                            &self.principal,
                            &self.workspace,
                            &self.host,
                            &self.instance,
                        )
                        .await
                }
                None => Err((
                    "OWNED_PROFILE_UNAVAILABLE".into(),
                    "Owned profile is unavailable".into(),
                )),
            };
            mcp::handle_with_tools(
                &input,
                "many-ai-cli-execution",
                &definitions,
                &mut |_, _| result.clone(),
            )
        } else {
            preliminary
        };
        if output.is_null() {
            return Response::bytes(202, "application/json", Vec::new()).no_store();
        }
        Response::json(200, &output).no_store()
    }

    fn call(&self, tool: &str, args: &Value, now: i64) -> ToolResult {
        let error = |code: &str| {
            (
                code.to_owned(),
                "Execution contract rejected request".to_owned(),
            )
        };
        if tool == "capabilities" {
            return Ok(self.capabilities());
        }
        if tool == "executor_request" {
            let id = args["request_id"]
                .as_str()
                .ok_or_else(|| error("INVALID_ARGUMENT"))?;
            return self
                .journal
                .lock()
                .map_err(|_| error("EXECUTOR_STORAGE"))?
                .query_request(&self.principal, &self.workspace, id)
                .map_err(|e| (e.code, e.detail));
        }
        let binding: Binding = serde_json::from_value(args["binding"].clone())
            .map_err(|_| error("INVALID_BINDING"))?;
        if binding.principal_id != self.principal
            || binding.workspace_id != self.workspace
            || binding.execution_host_id != self.host
        {
            return Err(error("PRINCIPAL_MISMATCH"));
        }
        if tool == "executor_evidence" {
            return self
                .owner_store
                .as_ref()
                .ok_or_else(|| error("OWNER_EVIDENCE_UNAVAILABLE"))?
                .lock()
                .map_err(|_| error("EXECUTOR_STORAGE"))?
                .query(&binding)
                .map_err(|e| (e.code, e.detail));
        }
        let mut journal = self.journal.lock().map_err(|_| error("EXECUTOR_STORAGE"))?;
        if tool == "executor_poll" {
            let previous = journal.query(&binding).map_err(|e| (e.code, e.detail))?;
            if let Some(backend) = &self.backend {
                // A lookup can reconcile a known operation; it never starts one.
                if let Ok(event) = backend.poll(&binding) {
                    return journal
                        .observe(&binding, event)
                        .map_err(|e| (e.code, e.detail));
                }
            }
            return Ok(previous);
        }
        let id = args["request_id"]
            .as_str()
            .ok_or_else(|| error("INVALID_ARGUMENT"))?;
        if tool == "executor_start" {
            if binding.executor_instance_id != self.instance {
                // Old operations remain queryable/replayable after a service
                // restart. A new intent cannot impersonate an old owner.
                if let Ok(previous) = journal.query_request(&self.principal, &self.workspace, id) {
                    let receipt = &previous["request_receipt"];
                    if receipt["binding"] == json!(binding) && receipt["operation"] == "start" {
                        return Ok(receipt.clone());
                    }
                    return Err(error("REQUEST_CONFLICT"));
                }
                return Err(error("STALE_INSTANCE"));
            }
            // Production adapters without owner evidence cannot be used as a
            // backdoor around Iris' capability gate, even by direct tool calls.
            if self.legacy_capabilities()["execution_ready"] != true
                || binding.provider != "synthetic"
            {
                return Err(error("EXECUTION_UNAVAILABLE"));
            }
            match journal
                .start(id, &binding, now)
                .map_err(|e| (e.code, e.detail))?
            {
                Acceptance::Replay(receipt) => Ok(receipt),
                Acceptance::New(receipt) => {
                    journal
                        .authorize_dispatch(&binding, now)
                        .map_err(|e| (e.code, e.detail))?;
                    let event = self.backend.as_ref().unwrap().start(&binding);
                    match event {
                        Ok(event) => {
                            journal
                                .observe(&binding, event)
                                .map_err(|e| (e.code, e.detail))?;
                        }
                        Err(_) => {
                            journal.observe(&binding,json!({"binding":binding,"event_kind":"disconnected","event_seq":1,"observed_at":now})).map_err(|e| (e.code,e.detail))?;
                            return Err(error("EXECUTION_UNKNOWN"));
                        }
                    }
                    Ok(receipt)
                }
            }
        } else if tool == "executor_stop" {
            match journal
                .stop(id, &binding, now)
                .map_err(|e| (e.code, e.detail))?
            {
                Acceptance::Replay(receipt) => Ok(receipt),
                Acceptance::New(receipt) => {
                    if let Some(backend) = &self.backend
                        && let Ok(event) = backend.stop(&binding, id)
                    {
                        return journal
                            .observe(&binding, event)
                            .map_err(|e| (e.code, e.detail));
                    }
                    Ok(receipt)
                }
            }
        } else {
            Err(error("EXECUTOR_TOOL"))
        }
    }
}

fn tools() -> Vec<Value> {
    let s = || json!({"type":"string","minLength":1});
    let mut tools = vec![
        mcp::tool("capabilities", json!({}), &[]),
        mcp::tool(
            "executor_request",
            json!({"request_id":s()}),
            &["request_id"],
        ),
    ];
    for name in [
        "executor_start",
        "executor_poll",
        "executor_stop",
        "executor_evidence",
    ] {
        let mut props = json!({"binding":{"type":"object"}});
        let mut required = vec!["binding"];
        if !["executor_poll", "executor_evidence"].contains(&name) {
            props["request_id"] = s();
            required.push("request_id");
        }
        tools.push(mcp::tool(name, props, &required));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::super::iris_execution_journal::Grant;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fake {
        starts: AtomicUsize,
    }
    impl ExecutionBackend for Fake {
        fn capabilities(&self) -> Value {
            json!({"synthetic":true,"external_effects":false,"fenced_stop_proof":true,"result_sha256":true})
        }
        fn start(&self, b: &Binding) -> ToolResult {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"binding":b,"event_kind":"accepted","event_seq":1,"observed_at":10}))
        }
        fn poll(&self, b: &Binding) -> ToolResult {
            Ok(json!({"binding":b,"event_kind":"running","event_seq":2,"observed_at":11}))
        }
        fn stop(&self, _: &Binding, _: &str) -> ToolResult {
            Err(("STOP_UNCONFIRMED".into(), "No proof".into()))
        }
    }
    fn binding() -> Binding {
        serde_json::from_value(json!({"principal_id":"iris-principal","workspace_id":"test-workspace","plan_id":"plan-a","plan_revision":1,"task_id":"task-a","instruction_revision":1,"instruction_hash":"a".repeat(64),"approval_id":"approval-a","approval_revision":1,"scope_hash":"b".repeat(64),"execution_host_id":"host-a","executor_instance_id":"worker-a","attempt_id":"attempt-a","fence":1,"provider":"synthetic","profile_id":"synthetic"})).unwrap()
    }
    fn fixture(granted: bool, enabled: bool) -> (tempfile::TempDir, IrisExecutionHttp, Arc<Fake>) {
        let temp = tempfile::tempdir().unwrap();
        let mut journal =
            Journal::open(&temp.path().join("journal.sqlite3"), "host-a", "boot-a").unwrap();
        if granted {
            journal
                .register_grant(Grant {
                    binding: binding(),
                    expires_at: 100,
                    revoked: false,
                })
                .unwrap();
        }
        let fake = Arc::new(Fake {
            starts: AtomicUsize::new(0),
        });
        let owner = IrisExecutionHttp::new(
            "c".repeat(64),
            "iris-principal".into(),
            "test-workspace".into(),
            "host-a".into(),
            "worker-a".into(),
            48765,
            journal,
            if enabled { Some(fake.clone()) } else { None },
        )
        .unwrap();
        (temp, owner, fake)
    }
    fn request(tool: &str, args: Value) -> Request {
        let body = mcp::request(1, "tools/call", json!({"name":tool,"arguments":args}));
        Request {
            method: "POST".into(),
            path: PATH.into(),
            host: "127.0.0.1:48765".into(),
            remote_addr: "127.0.0.1:43210".into(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", "c".repeat(64))),
                ("MCP-Protocol-Version".into(), mcp::VERSION.into()),
                ("Mcp-Method".into(), "tools/call".into()),
                ("Mcp-Name".into(), tool.into()),
                ("Content-Type".into(), "application/json".into()),
                (
                    "Accept".into(),
                    "application/json, text/event-stream".into(),
                ),
            ],
            body: serde_json::to_vec(&body).unwrap(),
            ..Default::default()
        }
    }
    fn data(owner: &IrisExecutionHttp, request: &Request) -> Value {
        let response = owner.handle_authenticated(request, 10);
        assert_eq!(response.status, 200);
        serde_json::from_slice::<Value>(&response.body).unwrap()["result"]["structuredContent"]
            .clone()
    }
    #[test]
    fn production_capability_is_honest_and_direct_start_cannot_bypass_it() {
        let (_temp, owner, fake) = fixture(true, false);
        assert_eq!(owner.capabilities()["durable_start_receipt"], true);
        for field in [
            "execution_ready",
            "fenced_stop_proof",
            "result_sha256",
            "live_provider_execution",
        ] {
            assert_eq!(owner.capabilities()[field], false);
        }
        let value = data(
            &owner,
            &request(
                "executor_start",
                json!({"request_id":"start-a","binding":binding()}),
            ),
        );
        assert_eq!(value["error"]["code"], "EXECUTION_UNAVAILABLE");
        assert_eq!(fake.starts.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn missing_proof_backend_and_stale_instance_cannot_start() {
        struct NoProof(Arc<Fake>);
        impl ExecutionBackend for NoProof {
            fn capabilities(&self) -> Value {
                json!({"synthetic":true,"external_effects":false,"fenced_stop_proof":false,"result_sha256":true})
            }
            fn start(&self, b: &Binding) -> ToolResult {
                self.0.start(b)
            }
            fn poll(&self, b: &Binding) -> ToolResult {
                self.0.poll(b)
            }
            fn stop(&self, b: &Binding, id: &str) -> ToolResult {
                self.0.stop(b, id)
            }
        }
        let (_temp, mut owner, fake) = fixture(true, true);
        let mut stale = binding();
        stale.executor_instance_id = "old-worker".into();
        assert_eq!(
            data(
                &owner,
                &request(
                    "executor_start",
                    json!({"request_id":"stale","binding":stale})
                )
            )["error"]["code"],
            "STALE_INSTANCE"
        );
        owner.backend = Some(Arc::new(NoProof(fake.clone())));
        assert_eq!(owner.capabilities()["execution_ready"], false);
        assert_eq!(
            data(
                &owner,
                &request(
                    "executor_start",
                    json!({"request_id":"no-proof","binding":binding()})
                )
            )["error"]["code"],
            "EXECUTION_UNAVAILABLE"
        );
        assert_eq!(fake.starts.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn authentication_has_no_cookie_query_or_trusted_network_fallback() {
        let (_temp, owner, _) = fixture(true, true);
        let original = request("capabilities", json!({}));
        for mode in 0..6 {
            let mut r = original.clone();
            match mode {
                0 => {
                    r.headers.retain(|(k, _)| k != "Authorization");
                    r.headers.push((
                        "Cookie".into(),
                        format!("MANY_AI_CLI_token={}", "c".repeat(64)),
                    ));
                }
                1 => {
                    r.query = format!("token={}", "c".repeat(64));
                }
                2 => {
                    r.headers
                        .push(("authorization".into(), format!("Bearer {}", "c".repeat(64))));
                }
                3 => {
                    r.host = "evil.example".into();
                }
                4 => {
                    r.headers
                        .push(("Origin".into(), "http://evil.example".into()));
                }
                _ => {
                    r.remote_addr = "192.0.2.10:43210".into();
                }
            }
            assert!(owner.preflight(&r).is_err(), "mode {mode}");
        }
        assert!(owner.preflight(&original).is_ok());
    }
    #[test]
    fn strict_parser_and_http_mirrors_prevent_mutation() {
        let (_temp, owner, fake) = fixture(true, true);
        for raw in [br#"{"x":1,"x":2}"#.as_slice(), b"{} {}", b"\xff"] {
            let mut r = request(
                "executor_start",
                json!({"request_id":"start-a","binding":binding()}),
            );
            r.body = raw.to_vec();
            assert_eq!(owner.handle_authenticated(&r, 10).status, 400);
        }
        let mut r = request(
            "executor_start",
            json!({"request_id":"start-a","binding":binding()}),
        );
        r.headers.push(("mcp-name".into(), "executor_poll".into()));
        assert_eq!(owner.handle_authenticated(&r, 10).status, 400);
        assert_eq!(fake.starts.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn principal_and_grant_are_checked_before_launch_and_replay_never_relaunches() {
        let (_temp, owner, fake) = fixture(false, true);
        let args = json!({"request_id":"start-a","binding":binding()});
        assert!(
            data(&owner, &request("executor_start", args.clone()))
                .get("error")
                .is_some()
        );
        assert_eq!(fake.starts.load(Ordering::SeqCst), 0);
        let (_temp, owner, fake) = fixture(true, true);
        let first = data(&owner, &request("executor_start", args.clone()));
        assert!(first.get("error").is_none(), "{first}");
        assert_eq!(first, data(&owner, &request("executor_start", args)));
        assert_eq!(fake.starts.load(Ordering::SeqCst), 1);
        let mut other = binding();
        other.principal_id = "other".into();
        assert_eq!(
            data(&owner, &request("executor_poll", json!({"binding":other})))["error"]["code"],
            "PRINCIPAL_MISMATCH"
        );
    }
    #[test]
    fn no_configuration_creates_no_state_and_bad_configuration_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            IrisExecutionHttp::from_local(temp.path(), 48765)
                .unwrap()
                .is_none()
        );
        assert!(!temp.path().join("iris-execution").exists());
        std::fs::create_dir(temp.path().join("iris-execution")).unwrap();
        std::fs::write(temp.path().join("iris-execution/identity.json"), b"{}").unwrap();
        assert!(IrisExecutionHttp::from_local(temp.path(), 48765).is_err());
        assert!(!temp.path().join("iris-execution/journal.sqlite3").exists());
    }
    #[test]
    fn credential_trailing_newline_is_accepted_and_aliased_credential_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("iris-execution");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("identity.json"),
            br#"{"principal_id":"iris-principal","workspace_id":"test-workspace","execution_host_id":"host-a"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("credential"), format!("{}\r\n", "c".repeat(64))).unwrap();
        assert!(
            IrisExecutionHttp::from_local(temp.path(), 48765)
                .unwrap()
                .is_some()
        );
        let outside = temp.path().join("outside-credential");
        std::fs::write(&outside, "d".repeat(64)).unwrap();
        std::fs::remove_file(dir.join("credential")).unwrap();
        std::fs::hard_link(&outside, dir.join("credential")).unwrap();
        assert!(IrisExecutionHttp::from_local(temp.path(), 48765).is_err());
    }
}
