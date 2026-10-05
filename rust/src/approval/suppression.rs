//! Corrupt-marker diagnostics and UI-notice admission. This is not answered
//! approval state: it never owns or changes a record, candidate, or epoch.
//!
//! Keep one value in the Session. Warm reattach carries it unchanged; only the
//! source history-reset operation clears it. Valid and absent markers do not
//! clear an earlier suppressed signature or notification time.
use super::marker::{self, Marker};
use crate::proto::time::Timestamp;
use std::time::Duration;

const GO_ZERO: Timestamp = match Timestamp::from_unix(-62_135_596_800, 0) {
    Ok(value) => value,
    Err(_) => panic!("Go zero time is representable"),
};
const NOTICE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerSuppressionState {
    suppressed_sig: String,
    suppressed_at: Timestamp,
}

impl Default for MarkerSuppressionState {
    fn default() -> Self {
        Self {
            suppressed_sig: String::new(),
            suppressed_at: GO_ZERO,
        }
    }
}

/// Immutable admission only. The caller retains the exact marker, provider,
/// source and detection time needed to construct its real diagnostic/UI effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerSuppressionDecision {
    Empty,
    Valid,
    Suppressed {
        reason: &'static str,
        /// Warn when true, if the application has a logger. Alternating corrupt
        /// signatures each warn even while the UI notice remains throttled.
        new_signature: bool,
        /// The source snapshot-probe call site is inside the Session lock;
        /// warning, dump-probe call site and UI notice follow outside that lock.
        /// The pinned Go source has no registered sink for either debug probe.
        /// This flag is eligibility, never evidence that a dump was produced.
        notify: bool,
        /// Raw newline count plus one, without ANSI stripping or normalization.
        lines: usize,
    },
}

impl MarkerSuppressionState {
    pub fn suppressed_sig(&self) -> &str {
        &self.suppressed_sig
    }

    pub fn suppressed_at(&self) -> Timestamp {
        self.suppressed_at
    }

    pub fn reset_history(&mut self) {
        *self = Self::default();
    }

    /// Called under the existing Session lock after its binding was validated.
    /// A missing Session must not acquire state or emit effects. Valid returns
    /// to the common question/ledger path; it does not imply a record was opened.
    pub fn evaluate(
        &mut self,
        marker: Option<&Marker>,
        detected_at: Timestamp,
    ) -> MarkerSuppressionDecision {
        let Some(marker) = marker.filter(|m| !m.block.is_empty() && !m.sig.is_empty()) else {
            return MarkerSuppressionDecision::Empty;
        };
        let reason = marker::classify(&marker.block);
        if reason.is_empty() {
            return MarkerSuppressionDecision::Valid;
        }
        let new_signature = self.suppressed_sig != marker.sig;
        // Go Time.IsZero checks the actual year-1 instant. A notice at zero
        // leaves that sentinel active. Earlier timestamps have a negative Sub,
        // which cannot satisfy >= 30s; do not substitute unsigned saturation.
        let notify = new_signature
            && (self.suppressed_at == GO_ZERO
                || detected_at
                    .duration_since(self.suppressed_at)
                    .is_ok_and(|elapsed| elapsed >= NOTICE_INTERVAL));
        self.suppressed_sig.clone_from(&marker.sig);
        if notify {
            self.suppressed_at = detected_at;
        }
        MarkerSuppressionDecision::Suppressed {
            reason,
            new_signature,
            notify,
            lines: marker.block.bytes().filter(|byte| *byte == b'\n').count() + 1,
        }
    }
}

#[cfg(test)]
mod tests;
