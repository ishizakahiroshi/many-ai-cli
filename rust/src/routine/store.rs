//! One durable owner for routine definitions, runs and request aliases.
use super::{
    model::{self, Definition, RoutineFile, Run, active},
    schedule, validation,
};
use crate::{
    files::safe_fs::Dir,
    proto::{
        DoneSummary,
        core::{DbSessionId, LiveSessionId},
    },
};
use std::time::SystemTime;
use std::{
    collections::BTreeMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Missing,
    Changed,
    Running,
    Limit,
    ScheduleChanged,
    LaunchUnavailable,
    Invalid(String),
    Operation,
}
#[derive(Clone, Debug)]
pub struct Admission {
    pub run: Run,
    pub existing: bool,
}
#[derive(Clone)]
pub struct Observation {
    pub id: LiveSessionId,
    pub db_id: Option<DbSessionId>,
    pub launch_label: String,
    pub state: String,
    pub waiting: bool,
    pub last_output: Option<SystemTime>,
}
type Writer = dyn Fn(&Dir, &[u8]) -> io::Result<()> + Send + Sync;
struct State {
    data: RoutineFile,
    unavailable: bool,
    missing_since: BTreeMap<String, SystemTime>,
    pending_results: BTreeMap<String, Run>,
}
pub struct RoutineStore {
    root: Arc<Dir>,
    state: Mutex<State>,
    write: Arc<Writer>,
}
impl RoutineStore {
    pub fn open(root: Arc<Dir>) -> Self {
        Self::with_writer(
            root,
            Arc::new(|dir, bytes| dir.replace("routines.json", bytes, 0o600)),
        )
    }
    fn with_writer(root: Arc<Dir>, write: Arc<Writer>) -> Self {
        let loaded = match root.read("routines.json", usize::MAX) {
            Ok(bytes) => RoutineFile::decode(&bytes).map_err(|_| ()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RoutineFile::default()),
            Err(_) => Err(()),
        };
        let unavailable = loaded.as_ref().is_err() || loaded.as_ref().is_ok_and(|d| d.version != 1);
        Self {
            root,
            write,
            state: Mutex::new(State {
                data: loaded.unwrap_or_default(),
                unavailable,
                missing_since: BTreeMap::new(),
                pending_results: BTreeMap::new(),
            }),
        }
    }
    pub fn ready(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unavailable
    }
    fn commit(&self, state: &mut State, next: RoutineFile) -> Result<(), Error> {
        if state.unavailable {
            return Err(Error::Unavailable);
        }
        let bytes = serde_json::to_vec_pretty(&next).map_err(|_| Error::Operation)?;
        (self.write)(&self.root, &bytes).map_err(|_| Error::Operation)?;
        state.data = next;
        Ok(())
    }
    pub fn definitions(&self) -> Result<Vec<Definition>, Error> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.unavailable {
            return Err(Error::Unavailable);
        }
        Ok(state.data.routines.clone())
    }
    pub fn run(&self, id: &str) -> Result<Option<Run>, Error> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.unavailable {
            return Err(Error::Unavailable);
        }
        Ok(state.data.runs.iter().find(|r| r.id == id).cloned())
    }
    pub fn runs(&self, filter: &str) -> Result<Vec<Run>, Error> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.unavailable {
            return Err(Error::Unavailable);
        }
        Ok(state
            .data
            .runs
            .iter()
            .rev()
            .filter(|r| filter.is_empty() || r.routine_id == filter)
            .take(200)
            .cloned()
            .collect())
    }
    pub fn save(
        &self,
        id: Option<&str>,
        mut item: Definition,
        home: &Path,
        now: SystemTime,
    ) -> Result<Definition, Error> {
        validation::validate(&mut item, home, now).map_err(Error::Invalid)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        let index = match id {
            Some(id) => Some(
                next.routines
                    .iter()
                    .position(|r| r.id == id)
                    .ok_or(Error::Missing)?,
            ),
            None => None,
        };
        if let Some(index) = index {
            if !item.updated_at.is_empty() && item.updated_at != next.routines[index].updated_at {
                return Err(Error::Changed);
            }
        } else if next.routines.len() >= 200 {
            return Err(Error::Limit);
        }
        item.updated_at = model::time(now)?;
        item.next_run_at = if item.enabled {
            schedule::next(&item.schedule, now)
                .map_err(Error::Invalid)?
                .map(model::time)
                .transpose()?
                .unwrap_or_default()
        } else {
            String::new()
        };
        if let Some(index) = index {
            item.id = next.routines[index].id.clone();
            item.created_at = next.routines[index].created_at.clone();
            next.routines[index] = item.clone();
        } else {
            item.id = random_id()?;
            item.created_at = item.updated_at.clone();
            next.routines.push(item.clone());
        }
        self.commit(&mut state, next)?;
        Ok(item)
    }
    pub fn delete(&self, id: &str) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        let index = next
            .routines
            .iter()
            .position(|r| r.id == id)
            .ok_or(Error::Missing)?;
        if next
            .runs
            .iter()
            .any(|r| r.routine_id == id && active(&r.status))
        {
            return Err(Error::Running);
        }
        next.routines.remove(index);
        self.commit(&mut state, next)
    }
    /// Persist the immutable launch snapshot before returning permission to launch.
    /// Exactly one caller receives existing=false for any active run/request alias.
    pub fn admit(
        &self,
        id: &str,
        request_id: &str,
        trigger: &str,
        now: SystemTime,
    ) -> Result<Admission, Error> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        let index = next
            .routines
            .iter()
            .position(|r| r.id == id)
            .ok_or(Error::Missing)?;
        let definition = &next.routines[index];
        if trigger == "schedule"
            && (!definition.enabled
                || definition.schedule.kind == "manual"
                || request_id != format!("schedule:{}", definition.next_run_at))
        {
            return Err(Error::ScheduleChanged);
        }
        let request_key = format!("{id}|{request_id}");
        if let Some(run) = next
            .runs
            .iter()
            .find(|r| {
                r.routine_id == id
                    && ((!request_id.is_empty()
                        && (r.request_id == request_id
                            || next.requests.get(&request_key) == Some(&r.id)))
                        || active(&r.status))
            })
            .cloned()
        {
            if !request_id.is_empty() && next.requests.get(&request_key) != Some(&run.id) {
                next.requests.insert(request_key, run.id.clone());
                self.commit(&mut state, next)?;
            }
            if state.unavailable {
                return Err(Error::Unavailable);
            }
            return Ok(Admission {
                run,
                existing: true,
            });
        }
        let run = new_run(definition, request_id, trigger, now)?;
        if trigger == "schedule" {
            next.routines[index].next_run_at = schedule::next(&definition.schedule, now)
                .map_err(Error::Invalid)?
                .map(model::time)
                .transpose()?
                .unwrap_or_default();
        }
        next.runs.push(run.clone());
        if !request_id.is_empty() {
            next.requests.insert(request_key, run.id.clone());
        }
        self.commit(&mut state, next)?;
        Ok(Admission {
            run,
            existing: false,
        })
    }
    pub fn launched(
        &self,
        id: &str,
        session: Result<LiveSessionId, ()>,
        instance: &str,
        now: SystemTime,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        if let Some(run) = next.runs.iter_mut().find(|r| r.id == id)
            && active(&run.status)
        {
            run.updated_at = model::time(now)?;
            match session {
                Ok(session) => {
                    run.session_id = session.0;
                    run.hub_instance_id = instance.into();
                    run.status = "running".into();
                }
                Err(()) => {
                    run.status = "failed".into();
                    run.error="The AI session could not be started. Check the provider installation and project directory.".into();
                    run.finished_at = run.updated_at.clone();
                }
            }
        }
        self.commit(&mut state, next)
    }
    pub fn skip_schedule(
        &self,
        definition: &Definition,
        now: SystemTime,
        reason: &str,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        let Some(index) = next
            .routines
            .iter()
            .position(|r| r.id == definition.id && r.next_run_at == definition.next_run_at)
        else {
            return Ok(());
        };
        let item = &mut next.routines[index];
        let due = schedule::next(&item.schedule, now).map_err(Error::Invalid)?;
        let mut run = new_run(
            item,
            &format!("schedule:{}", item.next_run_at),
            "schedule",
            now,
        )?;
        run.status = "skipped".into();
        run.error = reason.into();
        run.finished_at = run.started_at.clone();
        item.next_run_at = due.map(model::time).transpose()?.unwrap_or_default();
        next.runs.push(run);
        self.commit(&mut state, next)
    }
    /// Observation snapshots are obtained before acquiring this store's mutex.
    pub fn refresh(
        &self,
        observations: &[Observation],
        instance: &str,
        now: SystemTime,
    ) -> Result<(), Error> {
        let observations: BTreeMap<_, _> = observations
            .iter()
            .map(|o| (o.launch_label.as_str(), o))
            .collect();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        let mut changed = false;
        for run in &mut next.runs {
            if active(&run.status)
                && let Some(pending) = state.pending_results.get(&run.id)
            {
                *run = pending.clone();
                changed = true;
            }
            if !active(&run.status) {
                continue;
            }
            let before = run.clone();
            if let Some(obs) = observations
                .get(run.session_label.as_str())
                .filter(|o| o.state != "disconnected")
            {
                state.missing_since.remove(&run.id);
                run.session_id = obs.id.0;
                run.hub_instance_id = instance.into();
                if run.session_db_id == 0 {
                    run.session_db_id = obs.db_id.map(|id| id.0).unwrap_or(0);
                }
                if matches!(obs.state.as_str(), "completed" | "error") {
                    run.status = if obs.state == "error" {
                        "failed"
                    } else {
                        "finished"
                    }
                    .into();
                    run.finished_at = model::time(now)?;
                    if obs.state == "error" {
                        run.error = "The AI session ended with an error.".into();
                    }
                } else if obs.waiting {
                    run.status = "waiting".into();
                    run.error.clear();
                } else if obs.state == "standby"
                    && obs.last_output.is_some_and(|last| {
                        now.duration_since(last)
                            .is_ok_and(|d| d > Duration::from_secs(30))
                    })
                {
                    run.status = "waiting".into();
                    run.error="No completion result has been received. Open the session to confirm the outcome.".into();
                } else {
                    run.status = "running".into();
                    run.error.clear();
                }
            } else {
                let missing = *state.missing_since.entry(run.id.clone()).or_insert(now);
                if now
                    .duration_since(missing)
                    .is_ok_and(|d| d > Duration::from_secs(90))
                {
                    run.status = "interrupted".into();
                    run.finished_at = model::time(now)?;
                    run.error =
                        "The original session is unavailable. Its result has not been confirmed."
                            .into();
                }
            }
            if *run != before {
                run.updated_at = model::time(now)?;
                changed = true;
            }
        }
        if changed {
            self.commit(&mut state, next)?;
            state.pending_results.clear();
        }
        Ok(())
    }
    pub fn record_done(
        &self,
        label: &str,
        db_id: Option<DbSessionId>,
        summary: &DoneSummary,
        instance: &str,
    ) -> Result<(), Error> {
        if summary.fallback || label.is_empty() {
            return Ok(());
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.data.clone();
        let Some(run) = next
            .runs
            .iter_mut()
            .find(|r| r.session_label == label && active(&r.status))
        else {
            return Ok(());
        };
        run.session_id = summary.session_id;
        run.hub_instance_id = instance.into();
        if let Some(id) = db_id {
            run.session_db_id = id.0;
        }
        run.status = "finished".into();
        run.finished_at = summary.at.clone();
        run.updated_at = summary.at.clone();
        run.error.clear();
        let result = crate::storage::mask_secrets(&summary.text);
        run.summary = brief(&result);
        let mut end = result.len().min(256 * 1024);
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        run.result = result[..end].into();
        run.result_truncated = result.len() > end;
        run.result_available = !result.is_empty();
        let pending = run.clone();
        match self.commit(&mut state, next) {
            Ok(()) => {
                state.pending_results.remove(&pending.id);
                Ok(())
            }
            Err(error) => {
                state.pending_results.insert(pending.id.clone(), pending);
                Err(error)
            }
        }
    }
    pub fn active_session(&self, label: &str, at: SystemTime) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !state.unavailable
            && state.data.runs.iter().any(|r| {
                r.session_label == label
                    && active(&r.status)
                    && at
                        >= crate::proto::time::parse_rfc3339(&r.started_at).unwrap_or_else(|_| {
                            // Go ignores this historical parse error and compares
                            // against time.Time{} instead of rejecting the record.
                            crate::proto::time::parse_rfc3339("0001-01-01T00:00:00Z")
                                .expect("Go zero time is representable")
                        })
            })
    }
    pub fn run_url(&self, label: &str) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.unavailable || label.is_empty() {
            return None;
        }
        state
            .data
            .runs
            .iter()
            .find(|r| r.session_label == label)
            .map(|r| format!("/?routine_run={}", r.id))
    }
}
fn random_id() -> Result<String, Error> {
    crate::process::random_token()
        .map(|s| s[..32].into())
        .map_err(|_| Error::Operation)
}
fn new_run(def: &Definition, request: &str, trigger: &str, now: SystemTime) -> Result<Run, Error> {
    let id = random_id()?;
    let at = model::time(now)?;
    Ok(Run {
        session_label: format!("routine-{id}"),
        id,
        routine_id: def.id.clone(),
        routine_name: def.name.clone(),
        cwd: def.cwd.clone(),
        provider: def.provider.clone(),
        model: def.model.clone(),
        prompt: def.prompt.clone(),
        trigger: trigger.into(),
        status: "starting".into(),
        started_at: at.clone(),
        updated_at: at,
        request_id: request.into(),
        ..Default::default()
    })
}
fn brief(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 320 {
        text
    } else {
        format!("{}…", text.chars().take(320).collect::<String>().trim())
    }
}
#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
