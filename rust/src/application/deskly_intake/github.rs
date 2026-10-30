//! Read-only PR HEAD observation. One unauthenticated GET per check, so the
//! reader can never write and works only for repositories visible without a
//! credential. Private repositories need a credential decision first (see
//! README); this reader reports them as not visible instead of guessing.
use super::wire::valid_sha1;
use serde::Deserialize;
use std::time::Duration;

/// `owner/name` with GitHub's allowed characters only.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepoName(String);
impl RepoName {
    pub fn parse(raw: &str) -> Result<Self, &'static str> {
        let mut parts = raw.split('/');
        let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err("repository must be owner/name");
        };
        let part_ok = |part: &str| {
            (1..=100).contains(&part.len())
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if !part_ok(owner) || !part_ok(name) {
            return Err("invalid repository name");
        }
        Ok(Self(raw.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrHead {
    pub sha: String,
    pub open: bool,
    pub base_repo: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrHeadError {
    NotVisible,
    Unavailable,
    Malformed,
}
#[allow(async_fn_in_trait)]
pub trait PrHeadReader {
    async fn head(&self, repo: &RepoName, pull_request: u64) -> Result<PrHead, PrHeadError>;
}

pub struct GithubPrHeadReader {
    http: reqwest::Client,
    api_base: url::Url,
}
impl GithubPrHeadReader {
    pub fn public_api() -> Result<Self, &'static str> {
        Self::with_base("https://api.github.com/")
    }
    pub(super) fn with_base(base: &str) -> Result<Self, &'static str> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .user_agent("many-ai-cli-deskly-intake")
            .build()
            .map_err(|_| "http client")?;
        let api_base = url::Url::parse(base).map_err(|_| "api base")?;
        Ok(Self { http, api_base })
    }
}
#[derive(Deserialize)]
struct Pull {
    state: String,
    head: Ref,
    base: Base,
}
#[derive(Deserialize)]
struct Ref {
    sha: String,
}
#[derive(Deserialize)]
struct Base {
    repo: BaseRepo,
}
#[derive(Deserialize)]
struct BaseRepo {
    full_name: String,
}
impl PrHeadReader for GithubPrHeadReader {
    async fn head(&self, repo: &RepoName, pull_request: u64) -> Result<PrHead, PrHeadError> {
        if pull_request == 0 {
            return Err(PrHeadError::Malformed);
        }
        let url = self
            .api_base
            .join(&format!("repos/{}/pulls/{pull_request}", repo.as_str()))
            .map_err(|_| PrHeadError::Malformed)?;
        let response = self
            .http
            .get(url)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .send()
            .await
            .map_err(|_| PrHeadError::Unavailable)?;
        match response.status().as_u16() {
            200 => {}
            404 | 403 | 401 => return Err(PrHeadError::NotVisible),
            _ => return Err(PrHeadError::Unavailable),
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| PrHeadError::Unavailable)?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(PrHeadError::Malformed);
        }
        let pull: Pull = serde_json::from_slice(&bytes).map_err(|_| PrHeadError::Malformed)?;
        if !valid_sha1(&pull.head.sha) {
            return Err(PrHeadError::Malformed);
        }
        Ok(PrHead {
            sha: pull.head.sha,
            open: pull.state == "open",
            base_repo: pull.base.repo.full_name,
        })
    }
}
