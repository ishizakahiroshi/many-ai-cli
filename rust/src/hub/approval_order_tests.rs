use super::sockets::*;
use crate::proto::{self, core::*, time::UNIX_EPOCH};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
struct Owners {
    automatic: AtomicBool,
    persisted: Mutex<Vec<String>>,
    published: AtomicUsize,
}
impl OrderedEventObserver for Owners {
    fn approval_opened<'a>(
        &'a self,
        _: LiveSessionId,
        _: &'a ImmutableApprovalRecord,
    ) -> CoreFuture<'a, Result<bool, SessionError>> {
        Box::pin(async move { Ok(self.automatic.load(Ordering::SeqCst)) })
    }
    fn observe<'a>(&'a self, _: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
impl CoreEventPublisher for Owners {
    fn publish(&self, _: CoreEvent) -> Result<EventSequence, SessionError> {
        self.published.fetch_add(1, Ordering::SeqCst);
        Ok(EventSequence(1))
    }
}
impl PersistenceEffectSink for Owners {
    fn apply(&self, effect: PersistenceEffect) -> Result<(), SessionError> {
        if let PersistenceEffect::ApprovalDetected(detected) = effect {
            self.persisted.lock().unwrap().push(detected.candidate_key);
        }
        Ok(())
    }
    fn apply_bound(
        &self,
        _: SessionBinding,
        _: PersistenceBindingScope,
        effect: PersistenceEffect,
    ) -> Result<(), SessionError> {
        self.apply(effect)
    }
}
struct Ticket(Arc<AtomicUsize>);
impl PersistenceOrder for Ticket {
    fn wait(&self) -> CoreFuture<'_, ()> {
        Box::pin(async {})
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn record() -> ImmutableApprovalRecord {
    ImmutableApprovalRecord::new(ApprovalRecordData {
        candidate: CandidateIdentity {
            key: "candidate".into(),
            shape: "shape".into(),
            source_epoch: ApprovalSourceEpoch(2),
        },
        sig: "sig".into(),
        origin: "native".into(),
        source: "go_vt".into(),
        kind: "command".into(),
        block: "".into(),
        question: "run".into(),
        context: "".into(),
        options: vec![],
        summary: proto::ApprovalSummary::default(),
        detected_at: UNIX_EPOCH,
    })
}
fn detected() -> ApprovalDetected {
    ApprovalDetected {
        live_session_id: LiveSessionId(1),
        sig: "sig".into(),
        source: "go_vt".into(),
        kind: "command".into(),
        provider: "codex".into(),
        question: "run".into(),
        context: "".into(),
        block: "".into(),
        candidate_key: "candidate".into(),
        source_epoch: ApprovalSourceEpoch(2),
        options: vec![],
        detected_at: Some(UNIX_EPOCH),
    }
}
#[tokio::test]
async fn automatic_open_releases_bound_persistence_ticket_without_erasing_a_later_manual_batch() {
    let owner = Arc::new(Owners {
        automatic: AtomicBool::new(true),
        persisted: Mutex::default(),
        published: AtomicUsize::new(0),
    });
    let driver = EffectDriver::new(
        Arc::new(SocketRegistry::default()),
        owner.clone(),
        owner.clone(),
        owner.clone(),
    );
    let dropped = Arc::new(AtomicUsize::new(0));
    let binding = SessionBinding {
        session: LiveSessionId(1),
        incarnation: SessionIncarnation(1),
        wrapper: WrapperConnectionId(1),
    };
    driver
        .apply(CoreEffects(vec![
            CoreEffect::Notify(CoreEvent::ApprovalOpened {
                session: binding.session,
                record: record(),
            }),
            CoreEffect::PersistBound {
                binding,
                scope: PersistenceBindingScope::Incarnation,
                effect: PersistenceEffect::ApprovalDetected(detected()),
                order: Box::new(Ticket(dropped.clone())),
            },
            CoreEffect::Notify(CoreEvent::ApprovalPublished {
                session: binding.session,
                record: record(),
            }),
        ]))
        .await
        .unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(owner.persisted.lock().unwrap().is_empty());
    assert_eq!(owner.published.load(Ordering::SeqCst), 0);
    owner.automatic.store(false, Ordering::SeqCst);
    driver
        .apply(CoreEffects(vec![
            CoreEffect::Notify(CoreEvent::ApprovalOpened {
                session: binding.session,
                record: record(),
            }),
            CoreEffect::PersistBound {
                binding,
                scope: PersistenceBindingScope::Incarnation,
                effect: PersistenceEffect::ApprovalDetected(detected()),
                order: Box::new(Ticket(dropped.clone())),
            },
            CoreEffect::Notify(CoreEvent::ApprovalPublished {
                session: binding.session,
                record: record(),
            }),
        ]))
        .await
        .unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
    assert_eq!(*owner.persisted.lock().unwrap(), ["candidate"]);
    assert_eq!(owner.published.load(Ordering::SeqCst), 2);
}
