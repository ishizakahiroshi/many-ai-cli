//! Explicit display-only evaluation. Private input and upstream errors stay private.
use super::{
    http::{Request, Response},
    network,
};
use crate::{
    config::RuntimePaths,
    process::Cancellation,
    proto::{core::CoreFuture, wire},
};
use futures_util::StreamExt;
use std::{io, sync::Arc, time::Duration};
pub const PATH: &str = "/api/jev/evaluate";
const TYPES: &[(&str, &str)] = &[
    ("simple_edit", "A small, precisely scoped edit"),
    (
        "bugfix",
        "Find and fix a defect when deep investigation is not specified",
    ),
    ("feature", "Add a new user-visible behavior"),
    (
        "deep_debug",
        "Trace a difficult failure across components or platforms",
    ),
    ("architecture", "Compare or design system-level approaches"),
    ("research", "Gather and compare external information"),
    ("review", "Assess existing work without implementing it"),
];
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Text {
    text: String,
}
impl wire::GoWire for Text {
    const GO_TYPE: &'static str = "JevText";
    const SCHEMAS: &'static [wire::Schema] = &[wire::Schema {
        name: "JevText",
        fields: &[wire::Field {
            name: "text",
            kind: "string",
        }],
    }];
}
pub trait JevIo: Send + Sync {
    fn evaluate<'a>(
        &'a self,
        key: &'a str,
        text: &'a str,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<Vec<u8>>>;
}
pub struct NativeJev {
    paths: RuntimePaths,
}
impl NativeJev {
    pub fn new(paths: RuntimePaths) -> Self {
        Self { paths }
    }
}
impl JevIo for NativeJev {
    fn evaluate<'a>(
        &'a self,
        key: &'a str,
        text: &'a str,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::other("evaluation disabled in trial"));
            }
            let operation = async {
                let url = network::validate_url(
                    "https://api.typesafe.ai/v1/systemone",
                    network::Policy::ExternalHttps,
                )
                .map_err(io::Error::other)?;
                let host = url.host_str().unwrap();
                let addresses: Vec<_> = tokio::net::lookup_host((host, 443))
                    .await?
                    .map(|a| a.ip())
                    .collect();
                let target = network::validated_target(
                    url.clone(),
                    &addresses,
                    network::Policy::ExternalHttps,
                )
                .map_err(io::Error::other)?;
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .resolve_to_addrs(host, &target.addresses)
                    .build()
                    .map_err(io::Error::other)?;
                let criteria: TMap = TYPES.iter().copied().collect();
                let payload = serde_json::json!({"state":text,"model":"jev-latest","questions":{
                    "task_type":{"type":"choice","instructions":"Choose the main task actually requested. Favor the described work over labels such as 'easy' or 'architecture'.","criteria":criteria},
                    "complexity":{"type":"score","instructions":"Rate the likely scope and uncertainty from the request text alone. Do not assume unseen code is simple.","criteria":["1: Localized and clear, with little uncertainty","2: Bounded change with some unknowns","3: Multiple tradeoffs or substantial context needed","4: Broad investigation or high uncertainty"]},
                    "needs_strong_model":{"type":"noul","instructions":"Would a stronger model likely reduce the risk of task failure, based only on this request?","criteria":{"true":"A stronger model likely reduces failure risk","false":"There is no clear reason it would reduce failure risk"}}
                }});
                let response = client
                    .post(url)
                    .bearer_auth(key)
                    .json(&payload)
                    .send()
                    .await
                    .map_err(io::Error::other)?;
                if response.status() != reqwest::StatusCode::OK {
                    return Err(io::Error::other("evaluation status"));
                }
                let mut stream = response.bytes_stream();
                let mut bytes = vec![];
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(io::Error::other)?;
                    let remaining = 65536usize.saturating_sub(bytes.len());
                    bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                    if bytes.len() == 65536 {
                        break;
                    }
                }
                Ok(bytes)
            };
            tokio::select! {result=tokio::time::timeout(Duration::from_secs(10),operation)=>result.map_err(io::Error::other)?,_=cancel.cancelled()=>Err(io::Error::other("cancelled"))}
        })
    }
}
type TMap = std::collections::BTreeMap<&'static str, &'static str>;
pub struct JevHttp {
    key: String,
    io: Arc<dyn JevIo>,
}
impl JevHttp {
    pub fn configured(&self) -> bool {
        !self.key.trim().is_empty()
    }
    pub fn new(key: String, io: Arc<dyn JevIo>) -> Self {
        Self { key, io }
    }
    pub async fn handle_authenticated(&self, request: &Request, cancel: &Cancellation) -> Response {
        if self.key.trim().is_empty() {
            return Response::error(
                503,
                "jev_not_configured",
                "Set TYPESAFE_API_KEY in the Hub process environment",
            );
        }
        let bad = || Response::error(400, "bad_request", "Expected one JSON object with text");
        if request.body.len() > 8192 {
            return bad();
        }
        let members = match wire::decode_go_json_members(&request.body) {
            Ok(value) => value.unwrap_or_default(),
            Err(_) => return bad(),
        };
        for (name, _) in members {
            if !name.eq_ignore_ascii_case("text") {
                return bad();
            }
        }
        let text = match wire::decode::<Text>(&request.body) {
            Ok(v) => v.text,
            Err(_) => return bad(),
        };
        let text = text.trim();
        if text.is_empty() || text.len() > 4096 {
            return Response::error(400, "bad_request", "Text must be 1 to 4096 bytes");
        }
        match self
            .io
            .evaluate(self.key.trim(), text, cancel)
            .await
            .ok()
            .and_then(|bytes| parse_result(&bytes))
        {
            Some(value) => Response::json(200, &value),
            None => Response::error(
                502,
                "jev_failed",
                "Jev evaluation failed or returned an unexpected response",
            ),
        }
    }
}

macro_rules! dto {
    ($name:ident {$($field:ident:$ty:ty),*})=>{
        #[derive(Default,serde::Deserialize)] #[serde(default)]
        struct $name {$($field:$ty),*}
    };
}
dto!(Upstream {
    answers: Answers,
    usage: Usage,
    model: String
});
dto!(Answers {
    task_type: Choice,
    complexity: Score,
    needs_strong_model: Strong
});
dto!(Choice {
    r#type: String,
    choice: String,
    confidence: f64
});
dto!(Score {
    r#type: String,
    score: f64,
    confidence: f64
});
dto!(Strong {
    r#type: String,
    noul: f64
});
dto!(Usage { input_tokens: i64 });
impl wire::GoWire for Upstream {
    const GO_TYPE: &'static str = "JevUpstream";
    const SCHEMAS: &'static [wire::Schema] = &[
        wire::Schema {
            name: "JevUpstream",
            fields: &[
                wire::Field {
                    name: "answers",
                    kind: "JevAnswers",
                },
                wire::Field {
                    name: "usage",
                    kind: "JevUsage",
                },
                wire::Field {
                    name: "model",
                    kind: "string",
                },
            ],
        },
        wire::Schema {
            name: "JevAnswers",
            fields: &[
                wire::Field {
                    name: "task_type",
                    kind: "JevChoice",
                },
                wire::Field {
                    name: "complexity",
                    kind: "JevScore",
                },
                wire::Field {
                    name: "needs_strong_model",
                    kind: "JevStrong",
                },
            ],
        },
        wire::Schema {
            name: "JevChoice",
            fields: &[
                wire::Field {
                    name: "type",
                    kind: "string",
                },
                wire::Field {
                    name: "choice",
                    kind: "string",
                },
                wire::Field {
                    name: "confidence",
                    kind: "float64",
                },
            ],
        },
        wire::Schema {
            name: "JevScore",
            fields: &[
                wire::Field {
                    name: "type",
                    kind: "string",
                },
                wire::Field {
                    name: "score",
                    kind: "float64",
                },
                wire::Field {
                    name: "confidence",
                    kind: "float64",
                },
            ],
        },
        wire::Schema {
            name: "JevStrong",
            fields: &[
                wire::Field {
                    name: "type",
                    kind: "string",
                },
                wire::Field {
                    name: "noul",
                    kind: "float64",
                },
            ],
        },
        wire::Schema {
            name: "JevUsage",
            fields: &[wire::Field {
                name: "input_tokens",
                kind: "int",
            }],
        },
    ];
}
fn parse_result(bytes: &[u8]) -> Option<serde_json::Value> {
    let v = wire::decode_http_json::<Upstream>(bytes).ok()?;
    let a = v.answers;
    if a.task_type.r#type != "choice"
        || a.complexity.r#type != "score"
        || a.needs_strong_model.r#type != "noul"
        || !TYPES.iter().any(|(key, _)| *key == a.task_type.choice)
    {
        return None;
    }
    if ![
        a.task_type.confidence,
        a.complexity.confidence,
        a.needs_strong_model.noul,
    ]
    .iter()
    .all(|p| p.is_finite() && (0.0..=1.0).contains(p))
        || !a.complexity.score.is_finite()
        || !(0.0..=3.0).contains(&a.complexity.score)
        || v.usage.input_tokens < 0
    {
        return None;
    }
    Some(
        serde_json::json!({"task_type":a.task_type.choice,"task_type_confidence":a.task_type.confidence,"complexity":a.complexity.score+1.0,"complexity_confidence":a.complexity.confidence,"needs_strong_model_probability":a.needs_strong_model.noul,"input_tokens":v.usage.input_tokens,"model":v.model}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Forbidden;
    impl JevIo for Forbidden {
        fn evaluate<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: &'a Cancellation,
        ) -> CoreFuture<'a, io::Result<Vec<u8>>> {
            panic!("invalid requests must not consume evaluator")
        }
    }
    #[tokio::test]
    async fn configuration_precedes_body_and_unknown_duplicate_types_are_strict() {
        let req = Request {
            body: b"broken".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            JevHttp::new("".into(), Arc::new(Forbidden))
                .handle_authenticated(&req, &Cancellation::default())
                .await
                .status,
            503
        );
        for bytes in [
            b"{\"text\":\"ok\",\"unknown\":1}".as_slice(),
            b"{\"text\":1,\"text\":\"ok\"}",
            b"{\"text\":\"ok\"} {}",
            b"{\"text\":null}",
        ] {
            let req = Request {
                body: bytes.to_vec(),
                ..Default::default()
            };
            assert_eq!(
                JevHttp::new("synthetic".into(), Arc::new(Forbidden))
                    .handle_authenticated(&req, &Cancellation::default())
                    .await
                    .status,
                400
            );
        }
    }
    #[test]
    fn upstream_uses_go_fields_and_rejects_wrong_numeric_types_and_ranges() {
        let valid = serde_json::json!({"ANSWERS":{"task_type":{"type":"choice","choice":"review","confidence":0.5},"complexity":{"type":"score","score":2},"needs_strong_model":{"type":"noul","noul":0.75}},"usage":{"input_tokens":3},"model":"synthetic"});
        assert_eq!(
            parse_result(&serde_json::to_vec(&valid).unwrap()).unwrap()["complexity"],
            3.0
        );
        let mut invalid = valid.clone();
        invalid["usage"]["input_tokens"] = serde_json::json!(1.5);
        assert!(parse_result(&serde_json::to_vec(&invalid).unwrap()).is_none());
        invalid = valid;
        invalid["ANSWERS"]["complexity"]["score"] = serde_json::json!(4);
        assert!(parse_result(&serde_json::to_vec(&invalid).unwrap()).is_none());
    }
}
