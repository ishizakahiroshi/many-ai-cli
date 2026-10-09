//! Full-snapshot user preferences and shared-template optimistic concurrency.
//!
//! The parent Hub applies its existing token/PIN, Host and Origin guard. This
//! module owns no configuration state: ConfigStore serializes every save and
//! publishes only a successfully persisted snapshot.
use super::http::{JSON_BODY_LIMIT, Request, Response, require_method};
use crate::{
    config::{
        self, Config, ConfigError, ConfigSnapshot, ConfigStore, Resource, RuntimePaths, UserPrefs,
        UserPrefsTemplate,
    },
    proto::{provider::to_go_json, unicode::simple_lower},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const PATH: &str = "/api/user-prefs";

/// `has_push_subscription` reads the existing push manager's subscription count;
/// an absent manager supplies false. It must not send notifications or fetch any
/// remote data. Avatar/sound upload and download routes are deliberately separate.
pub fn handle_authenticated(
    request: &Request,
    store: &ConfigStore,
    paths: &RuntimePaths,
    has_push_subscription: impl Fn() -> bool,
) -> Option<Response> {
    if request.path != PATH {
        return None;
    }
    if let Err(error) = require_method(request, &["GET", "PUT"]) {
        return Some(error);
    }
    Some(if request.method == "GET" {
        get(store, has_push_subscription)
    } else {
        put(request, store, paths)
    })
}

fn get(store: &ConfigStore, has_push_subscription: impl Fn() -> bool) -> Response {
    let snapshot = match store.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return save_error(error),
    };
    let mut prefs = snapshot.config.user_prefs;
    if prefs.done_summary_notify.enabled.is_none() {
        prefs.done_summary_notify.enabled = Some(
            snapshot
                .config
                .notify
                .backends
                .as_ref()
                .is_some_and(|backends| !backends.is_empty())
                || has_push_subscription(),
        );
    }
    prefs_response(&prefs)
}

fn put(request: &Request, store: &ConfigStore, paths: &RuntimePaths) -> Response {
    // One Go Decoder.Decode call, including its first-value-only semantics.
    let mut prefs = match config::decode_user_prefs_http_json(
        &request.body[..request.body.len().min(JSON_BODY_LIMIT)],
    ) {
        Ok(prefs) => prefs,
        Err(_) => return Response::error(400, "bad_request", "invalid json"),
    };
    sanitize(&mut prefs, paths);
    commit_preferences(request, store, prefs, |revision, next| {
        store.persist(revision, next)
    })
}

/// A revision race is not itself a template conflict. Re-read the canonical
/// configuration and redo preserve/compare after any concurrent config writer.
/// `persist` checks the revision and holds its lock through save/publication;
/// consequently no request can compare against an uncommitted template version.
fn commit_preferences(
    request: &Request,
    store: &ConfigStore,
    prefs: UserPrefs,
    mut persist: impl FnMut(u64, Config) -> Result<ConfigSnapshot, ConfigError>,
) -> Response {
    loop {
        let mut snapshot = match store.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => return save_error(error),
        };
        let mut next_prefs = prefs.clone();
        match request.header("X-Template-Write") {
            "preserve" => next_prefs.templates = snapshot.config.user_prefs.templates.clone(),
            "compare"
                if request.header("X-Template-Version")
                    != template_version(&snapshot.config.user_prefs.templates) =>
            {
                return Response::error(
                    409,
                    "template_conflict",
                    "shared templates changed on another device",
                );
            }
            _ => {}
        }
        snapshot.config.user_prefs = next_prefs;
        match persist(snapshot.revision, snapshot.config) {
            Ok(committed) => return prefs_response(&committed.config.user_prefs),
            Err(ConfigError::Conflict { .. }) => continue,
            Err(error) => return save_error(error),
        }
    }
}

fn prefs_response(prefs: &UserPrefs) -> Response {
    let mut response = Response::json(200, &prefs.public_json());
    response.headers.insert(
        "X-Template-Version".into(),
        template_version(&prefs.templates),
    );
    response
}
fn save_error(error: ConfigError) -> Response {
    Response::error(500, "save_failed", &format!("save failed: {error}"))
}

/// Do not hash Value/public_json: map sorting changes the Go struct field order.
/// This immutable view uses the existing typed templates as its only authority.
#[derive(Serialize)]
struct TemplateWire<'a> {
    label: &'a str,
    body: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    providers: &'a [String],
    #[serde(skip_serializing_if = "slice_is_empty")]
    tags: &'a [String],
}
fn slice_is_empty<T>(slice: &&[T]) -> bool {
    slice.is_empty()
}
pub fn template_version(templates: &[UserPrefsTemplate]) -> String {
    let wire: Vec<_> = templates
        .iter()
        .map(|template| TemplateWire {
            label: &template.label,
            body: &template.body,
            providers: &template.providers,
            tags: &template.tags,
        })
        .collect();
    let bytes = to_go_json(&wire).expect("template strings are JSON-compatible");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sanitize(prefs: &mut UserPrefs, paths: &RuntimePaths) {
    prefs.avatar = sanitize_avatar(&prefs.avatar, paths);
    let sound = &mut prefs.notify_sound;
    if !sound.custom_file.is_empty() || !sound.custom_mime.is_empty() {
        let expected = paths
            .resource(Resource::NotifySound)
            .to_string_lossy()
            .into_owned();
        if candidate_key(&sound.custom_file, cfg!(windows))
            == candidate_key(&expected, cfg!(windows))
            && notify_sound_allowed(&sound.custom_mime)
        {
            sound.custom_file = expected;
        } else {
            sound.custom_file.clear();
            sound.custom_mime.clear();
        }
    }
    prefs.display.custom_themes = config::sanitize_custom_themes(&prefs.display.custom_themes);
    prefs.display.theme =
        config::sanitize_display_theme(&prefs.display.theme, &prefs.display.custom_themes);
}
fn sanitize_avatar(raw: &str, paths: &RuntimePaths) -> String {
    let avatar = raw.trim();
    if avatar.is_empty() || avatar.starts_with("http://") || avatar.starts_with("https://") {
        return avatar.into();
    }
    let expected = paths
        .resource(Resource::Avatar)
        .to_string_lossy()
        .into_owned();
    if candidate_key(avatar, cfg!(windows)) == candidate_key(&expected, cfg!(windows)) {
        avatar.into()
    } else {
        String::new()
    }
}
fn candidate_key(path: &str, windows: bool) -> String {
    let clean = super::preference_media::clean_native_path(path, windows);
    if windows { simple_lower(&clean) } else { clean }
}
fn notify_sound_allowed(raw: &str) -> bool {
    let mime = simple_lower(raw.trim());
    mime.split_once(';')
        .map_or(mime.as_str(), |(base, _)| base.trim())
        .starts_with("audio/")
}

#[cfg(test)]
mod tests;
