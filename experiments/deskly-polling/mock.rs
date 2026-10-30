//! Offline experiment only. No Hub registration, network, process launch or persistence.
use std::collections::HashSet;
use std::time::Duration;

const DEFAULT_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct Work {
    id: &'static str, generation: u64, device: &'static str, repo: &'static str,
    sha: &'static str, deadline: u64, approved: bool, read_only: bool,
}
enum Ack { Accepted, Unknown }
trait Deskly {
    fn poll(&mut self) -> Result<Vec<Work>, ()>;
    fn claim(&mut self, work: &Work) -> bool;
    fn ack(&mut self, work: &Work) -> Ack;
}
trait Reader { fn read_pr(&mut self, work: &Work); }

// Volatile simulation state is deliberately not a restart-safe deduplication ledger.
struct Adapter { interval: Duration, seen: HashSet<(&'static str, u64)> }
impl Adapter {
    fn tick(&mut self, source: &mut impl Deskly, reader: &mut impl Reader,
            device: &str, repo: &str, current_sha: &str, now: u64, local_approval: bool) {
        let Ok(items) = source.poll() else { return };
        for w in items {
            if !w.approved || !w.read_only || !local_approval || w.device != device ||
               w.repo != repo || w.sha != current_sha || w.deadline <= now || w.generation == 0 {
                continue;
            }
            // Unknown claim or ACK must never be treated as safe to retry.
            if !self.seen.insert((w.id, w.generation)) { continue; }
            if !source.claim(&w) { continue; }
            if let Ack::Accepted = source.ack(&w) { reader.read_pr(&w); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mock { work: Work, claims: usize, acks: usize, unknown: bool, offline: bool }
    impl Deskly for Mock {
        fn poll(&mut self) -> Result<Vec<Work>, ()> { if self.offline { Err(()) } else { Ok(vec![self.work.clone()]) } }
        fn claim(&mut self, _: &Work) -> bool { self.claims += 1; true }
        fn ack(&mut self, _: &Work) -> Ack { self.acks += 1; if self.unknown { Ack::Unknown } else { Ack::Accepted } }
    }
    #[derive(Default)] struct MockReader(usize);
    impl Reader for MockReader { fn read_pr(&mut self, _: &Work) { self.0 += 1; } }
    fn fixture() -> (Adapter, Mock, MockReader) {
        (Adapter { interval: DEFAULT_INTERVAL, seen: HashSet::new() }, Mock {
            work: Work { id:"synthetic", generation:1, device:"device", repo:"example/repo",
                sha:"0123456789012345678901234567890123456789", deadline:100, approved:true, read_only:true },
            claims:0, acks:0, unknown:false, offline:false }, MockReader::default())
    }
    fn tick(a: &mut Adapter, m: &mut Mock, r: &mut MockReader, approval: bool) {
        a.tick(m, r, "device", "example/repo", "0123456789012345678901234567890123456789", 10, approval);
    }
    #[test] fn approved_work_is_read_once() {
        let (mut a, mut m, mut r) = fixture(); assert_eq!(a.interval.as_secs(), 300);
        tick(&mut a,&mut m,&mut r,true); tick(&mut a,&mut m,&mut r,true);
        assert_eq!((m.claims,m.acks,r.0),(1,1,1));
    }
    #[test] fn unknown_ack_is_held() {
        let (mut a,mut m,mut r)=fixture(); m.unknown=true;
        tick(&mut a,&mut m,&mut r,true); tick(&mut a,&mut m,&mut r,true);
        assert_eq!((m.claims,m.acks,r.0),(1,1,0));
    }
    #[test] fn invalid_or_offline_work_never_claims() {
        for case in 0..8 {
            let (mut a,mut m,mut r)=fixture();
            match case { 0=>m.work.device="other", 1=>m.work.repo="other/repo", 2=>m.work.sha="changed",
                3=>m.work.deadline=10, 4=>m.work.approved=false, 5=>m.work.read_only=false,
                6=>m.offline=true, _=>m.work.generation=0 }
            tick(&mut a,&mut m,&mut r,true); assert_eq!((m.claims,r.0),(0,0));
        }
        let (mut a,mut m,mut r)=fixture(); tick(&mut a,&mut m,&mut r,false); assert_eq!(m.claims,0);
    }
}
