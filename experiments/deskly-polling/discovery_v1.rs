//! Standalone, offline interoperability contract for Deskly pc_relay_discovery_v1.
//! No HTTP, credentials, Hub integration, process launch, GitHub reads or persisted state.
use std::collections::HashSet;

const PROFILE: &str = "pc_relay_discovery_v1";
const SHA: &str = "0123456789012345678901234567890123456789";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    request_id: &'static str,
    expected_revision: u64,
    generation: u64,
    authorization_epoch: u64,
    session_id: &'static str,
}
#[derive(Clone)]
struct Discovery {
    profile: &'static str,
    work: Vec<Candidate>,
    truncated: bool,
}
/// Independently authenticated, immutable read authorization. NOT returned by discovery.
#[derive(Clone)]
struct VerifiedRead {
    request_id: &'static str,
    expected_revision: u64,
    generation: u64,
    authorization_epoch: u64,
    session_id: &'static str,
    repo: &'static str,
    pull_request: u64,
    head_sha: &'static str,
    deadline: u64,
    authenticated: bool,
    read_only: bool,
}
struct LocalGuard {
    allowed_repo: &'static str,
    current_head_sha: &'static str,
    session_id: &'static str,
    now: u64,
    explicit_consent: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Consumption { Consumed, Denied, Unknown }
trait Authority {
    /// Mock of the complete, currently authenticated claim/handoff/issued-validation gate.
    /// There is NO corresponding single boolean or new lease API in live Deskly.
    fn consume_checked(&mut self, candidate: &Candidate) -> Consumption;
}
trait Reader { fn read_summary(&mut self, verified: &VerifiedRead); }

struct PollingBoundary {
    // Volatile safety hint ONLY; this does not survive crashes and cannot replace
    // the existing Deskly one-time authority or local pc-relay pending journal.
    seen: HashSet<(&'static str, u64)>,
}
fn valid_sha(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn matches(c: &Candidate, p: &VerifiedRead, g: &LocalGuard) -> bool {
    c.generation == 1 && c.expected_revision > 0 && c.authorization_epoch > 0
        && c.session_id == g.session_id
        && c.request_id == p.request_id
        && c.expected_revision == p.expected_revision
        && c.generation == p.generation
        && c.authorization_epoch == p.authorization_epoch
        && c.session_id == p.session_id
        && p.authenticated && p.read_only && p.pull_request > 0
        && p.repo == g.allowed_repo && p.head_sha == g.current_head_sha
        && valid_sha(p.head_sha) && p.deadline > g.now && g.explicit_consent
}
impl PollingBoundary {
    fn tick(
        &mut self,
        discovery: &Discovery,
        verified: Option<&VerifiedRead>,
        guard: &LocalGuard,
        authority: &mut impl Authority,
        reader: &mut impl Reader,
    ) -> usize {
        // No cursor or pagination has been verified for this API. Fail closed
        // on partial batches instead of ignoring potentially eligible work.
        if discovery.profile != PROFILE || discovery.truncated || discovery.work.len() > 16 {
            return 0;
        }
        let mut reads = 0;
        for candidate in &discovery.work {
            let Some(proof) = verified else { continue };
            if !matches(candidate, proof, guard) { continue };
            let key = (candidate.request_id, candidate.generation);
            if !self.seen.insert(key) { continue };
            // Unknown never means "retry": a new attempt requires reconciliation.
            if authority.consume_checked(candidate) != Consumption::Consumed { continue };
            reader.read_summary(proof);
            reads += 1;
        }
        reads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "00000000-0000-4000-8000-000000000001";
    fn candidate() -> Candidate {
        Candidate { request_id: ID, expected_revision: 3, generation: 1,
            authorization_epoch: 2, session_id: "worker-observed-session" }
    }
    fn discovery() -> Discovery {
        Discovery { profile: PROFILE, work: vec![candidate()], truncated: false }
    }
    fn proof() -> VerifiedRead {
        VerifiedRead { request_id: ID, expected_revision: 3, generation: 1,
            authorization_epoch: 2, session_id: "worker-observed-session",
            repo: "example/repo", pull_request: 51, head_sha: SHA,
            deadline: 100, authenticated: true, read_only: true }
    }
    fn guard() -> LocalGuard {
        LocalGuard { allowed_repo: "example/repo", current_head_sha: SHA,
            session_id: "worker-observed-session", now: 10, explicit_consent: true }
    }
    #[derive(Default)] struct FakeAuthority { calls: usize, consumed: bool, unknown: bool }
    impl Authority for FakeAuthority {
        fn consume_checked(&mut self, _: &Candidate) -> Consumption {
            self.calls += 1;
            if self.unknown { Consumption::Unknown }
            else if self.consumed { Consumption::Denied }
            else { self.consumed = true; Consumption::Consumed }
        }
    }
    #[derive(Default)] struct FakeReader(usize);
    impl Reader for FakeReader { fn read_summary(&mut self, _: &VerifiedRead) { self.0 += 1; } }
    fn tick(a: &mut PollingBoundary, d: &Discovery, p: Option<&VerifiedRead>, g: &LocalGuard,
        authority: &mut FakeAuthority, reader: &mut FakeReader) -> usize {
        a.tick(d, p, g, authority, reader)
    }
    #[test] fn only_matching_verified_read_with_local_consent_consumes_once() {
        let mut a = PollingBoundary { seen: HashSet::new() };
        let (d,p,g)=(discovery(),proof(),guard());
        let (mut authority,mut reader)=(FakeAuthority::default(),FakeReader::default());
        assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),1);
        assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),0);
        assert_eq!((authority.calls,reader.0),(1,1));
    }
    #[test] fn advisory_discovery_without_separate_verified_target_cannot_launch() {
        let (mut a,mut authority,mut reader)=(PollingBoundary{seen:HashSet::new()},FakeAuthority::default(),FakeReader::default());
        assert_eq!(tick(&mut a,&discovery(),None,&guard(),&mut authority,&mut reader),0);
        assert_eq!((authority.calls,reader.0),(0,0));
    }
    #[test] fn mismatched_generation_revision_epoch_or_session_is_denied() {
        for index in 0..5 {
            let mut d=discovery();
            let p=proof(); let g=guard();
            match index {
                0 => d.work[0].generation=2,
                1 => d.work[0].expected_revision=4,
                2 => d.work[0].authorization_epoch=3,
                3 => d.work[0].session_id="wrong",
                _ => d.work[0].expected_revision=0,
            }
            let (mut a,mut authority,mut reader)=(PollingBoundary{seen:HashSet::new()},FakeAuthority::default(),FakeReader::default());
            assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),0);
            assert_eq!(authority.calls,0);
        }
    }
    #[test] fn missing_consent_expired_or_unverified_read_scope_is_denied() {
        for index in 0..8 {
            let d=discovery();let mut p=proof();let mut g=guard();
            match index {
                0 => p.authenticated=false,
                1 => p.read_only=false,
                2 => p.repo="other/repo",
                3 => p.head_sha="changed",
                4 => p.deadline=10,
                5 => p.pull_request=0,
                6 => g.explicit_consent=false,
                _ => g.session_id="other-session",
            }
            let (mut a,mut authority,mut reader)=(PollingBoundary{seen:HashSet::new()},FakeAuthority::default(),FakeReader::default());
            assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),0);
            assert_eq!(authority.calls,0);
        }
    }
    #[test] fn truncated_unknown_profile_and_oversized_batches_are_not_launchable() {
        for index in 0..3 {
            let mut d=discovery();
            match index {
                0 => d.truncated=true,
                1 => d.profile="unknown",
                _ => d.work=vec![candidate();17],
            }
            let (mut a,mut authority,mut reader)=(PollingBoundary{seen:HashSet::new()},FakeAuthority::default(),FakeReader::default());
            assert_eq!(tick(&mut a,&d,Some(&proof()),&guard(),&mut authority,&mut reader),0);
            assert_eq!(authority.calls,0);
        }
    }
    #[test] fn unknown_authority_outcome_is_never_blindly_retried() {
        let (mut a,mut authority,mut reader)=(PollingBoundary{seen:HashSet::new()},FakeAuthority{unknown:true,..Default::default()},FakeReader::default());
        let (d,p,g)=(discovery(),proof(),guard());
        assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),0);
        authority.unknown=false;
        assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),0);
        assert_eq!((authority.calls,reader.0),(1,0));
    }
    #[test] fn two_mock_clients_share_only_one_authoritative_consumption() {
        let (mut a,mut b)=(PollingBoundary{seen:HashSet::new()},PollingBoundary{seen:HashSet::new()});
        let (mut authority,mut reader)=(FakeAuthority::default(),FakeReader::default());
        let (d,p,g)=(discovery(),proof(),guard());
        assert_eq!(tick(&mut a,&d,Some(&p),&g,&mut authority,&mut reader),1);
        assert_eq!(tick(&mut b,&d,Some(&p),&g,&mut authority,&mut reader),0);
        assert_eq!((authority.calls,reader.0),(2,1));
    }
}
