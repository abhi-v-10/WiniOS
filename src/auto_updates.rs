//! One background worker owns the opt-in schedule and durable notification history.
use crate::{settings, updates};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

const INTERVAL: u64 = 24 * 60 * 60;
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    last_attempt: Option<u64>,
    #[serde(default)]
    notified_versions: Vec<String>,
}
fn now() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
fn remaining(state: &State, now: u64) -> u64 {
    state.last_attempt.map_or(0, |last| {
        if last > now {
            INTERVAL
        } else {
            INTERVAL.saturating_sub(now - last)
        }
    })
}
fn delay(state: &State, wall: u64, elapsed: Option<Duration>) -> u64 {
    remaining(state, wall).max(elapsed.map_or(0, |d| INTERVAL.saturating_sub(d.as_secs())))
}
fn record_version(state: &mut State, version: &str) -> bool {
    if state.notified_versions.iter().any(|v| v == version) || state.notified_versions.len() >= 256
    {
        return false;
    }
    state.notified_versions.push(version.to_owned());
    true
}
fn read(path: &Path) -> Result<(State, Option<Vec<u8>>), &'static str> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((State::default(), None)),
        Err(_) => Err("Automatic update preferences are inaccessible."),
        Ok(_) => {
            let bytes = settings::read_bounded(path)?;
            let state = serde_json::from_slice(&bytes).map_err(
                |_| "Automatic update preferences are invalid; they were not overwritten.",
            )?;
            Ok((state, Some(bytes)))
        }
    }
}
fn save(path: &Path, old: &mut Option<Vec<u8>>, state: &State) -> Result<(), &'static str> {
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|_| "Could not save update preferences.")?;
    settings::atomic_save(path, old.as_deref(), &bytes)?;
    *old = Some(bytes);
    Ok(())
}
pub enum Event {
    State { enabled: bool, configured: bool },
    Release(updates::Release, bool),
    Error(&'static str),
}
enum Command {
    Set(bool),
    Refresh,
    Stop,
}
pub struct Worker {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
    allow_results: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn new(path: PathBuf, parent: HWND) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let allow_results = Arc::new(AtomicBool::new(true));
        let permitted = allow_results.clone();
        let hwnd = parent.0 as isize;
        let handle=std::thread::Builder::new().name("winios-updates".into()).spawn(move || {
            let file=path.with_file_name("update-notifications.json");
            let emit=|event| { let _=events.send(event); unsafe { let _=PostMessageW(Some(HWND(hwnd as *mut _)),crate::fluent::WAKE,WPARAM(0),LPARAM(0)); } };
            let (mut state,mut raw)=match read(&file) { Ok(value)=>value,Err(error)=>{emit(Event::Error(error));return;} };
            let mut configured=updates::source_configured(&path);
            let mut last_run:Option<Instant>=None;
            emit(Event::State{enabled:state.enabled,configured});
            loop {
                if stop.load(Ordering::Acquire) { break; }
                if state.enabled && configured && now().is_some_and(|t|delay(&state,t,last_run.map(|i|i.elapsed()))==0) {
                    let Some(at)=now() else { break; };
                    let mut next=state.clone();next.last_attempt=Some(at);
                    if save(&file,&mut raw,&next).is_ok() {
                        state=next;
                        last_run=Some(Instant::now());
                        if let Ok(Some(release))=updates::check(&path,&stop) {
                            if !stop.load(Ordering::Acquire) && permitted.load(Ordering::Acquire) && updates::source_still_matches(&path,&release) {
                                let version=release.notification_version();
                                let mut notice=false;
                                if let Some(version)=version {
                                    let mut next=state.clone();
                                    if record_version(&mut next,&version) {
                                        if save(&file,&mut raw,&next).is_ok() { state=next;notice=true; }
                                    }
                                }
                                emit(Event::Release(release,notice));
                            }
                        }
                    } else { state.enabled=false; emit(Event::Error("Automatic update preferences changed or could not be saved. Restart WiniOS after reviewing them."));emit(Event::State{enabled:false,configured}); }
                }
                let wait=if state.enabled && configured { Duration::from_secs(now().map_or(INTERVAL,|t|delay(&state,t,last_run.map(|i|i.elapsed()))).max(1)) } else { Duration::from_secs(INTERVAL) };
                match commands.recv_timeout(wait) {
                    Ok(Command::Stop)|Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                    Ok(Command::Refresh)=> { configured=updates::source_configured(&path);emit(Event::State{enabled:state.enabled,configured}); },
                    Ok(Command::Set(enabled))=> {
                        configured=updates::source_configured(&path);
                        if enabled && !configured { emit(Event::Error("Configure a trusted GitHub release source before enabling automatic checks.")); }
                        else { let mut next=state.clone();next.enabled=enabled;match save(&file,&mut raw,&next) { Ok(())=>{state=next;permitted.store(enabled,Ordering::Release);},Err(error)=>emit(Event::Error(error)) } }
                        emit(Event::State{enabled:state.enabled,configured});
                    },
                    Err(mpsc::RecvTimeoutError::Timeout)=>{configured=updates::source_configured(&path);}
                }
            }
        }).ok();
        Self {
            tx,
            rx,
            cancel,
            allow_results,
            handle,
        }
    }
    pub fn set(&self, enabled: bool) {
        if !enabled {
            self.allow_results.store(false, Ordering::Release);
        }
        let _ = self.tx.send(Command::Set(enabled));
    }
    pub fn refresh(&self) {
        let _ = self.tx.send(Command::Refresh);
    }
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok().map(|event| match event {
            Event::Release(release, notice) => Event::Release(
                release,
                notice && self.allow_results.load(Ordering::Acquire),
            ),
            other => other,
        })
    }
    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        let _ = self.tx.send(Command::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_is_bounded_across_restarts_and_clock_changes() {
        let mut s = State::default();
        assert_eq!(remaining(&s, 100), 0);
        s.last_attempt = Some(100);
        assert_eq!(remaining(&s, 101), 86399);
        assert_eq!(remaining(&s, 86500), 0);
        assert_eq!(remaining(&s, 99), INTERVAL);
        assert_eq!(
            delay(&s, 1000000, Some(Duration::from_secs(30))),
            INTERVAL - 30
        );
    }
    #[test]
    fn notification_history_deduplicates_nonconsecutive_versions_and_fails_closed_when_full() {
        let mut s = State::default();
        assert!(record_version(&mut s, "2.0.0"));
        assert!(record_version(&mut s, "3.0.0"));
        assert!(!record_version(&mut s, "2.0.0"));
        for i in 0..254 {
            assert!(record_version(&mut s, &format!("4.0.{i}")));
        }
        assert!(!record_version(&mut s, "5.0.0"));
        assert_eq!(s.notified_versions.len(), 256);
    }
    #[test]
    fn absent_preferences_default_off_and_unknown_fields_fail_closed() {
        let p = std::env::temp_dir().join(format!(
            "winios-auto-test-{}-{}.json",
            std::process::id(),
            now().unwrap()
        ));
        let (state, raw) = read(&p).unwrap();
        assert!(!state.enabled && raw.is_none());
        std::fs::write(&p, br#"{"enabled":true,"extra":1}"#).unwrap();
        assert!(read(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn unconfigured_worker_refuses_opt_in_without_creating_state_or_network_request() {
        let dir = std::env::temp_dir().join(format!(
            "winios-auto-worker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut worker = Worker::new(dir.join("settings.json"), HWND::default());
        worker.set(true);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut rejected = false;
        while Instant::now() < deadline {
            if let Some(Event::Error(message)) = worker.poll() {
                if message.contains("trusted GitHub") {
                    rejected = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        worker.stop();
        assert!(rejected);
        assert!(!dir.join("update-notifications.json").exists());
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn schedule_state_round_trips_atomically_and_detects_conflicting_edits() {
        let dir = std::env::temp_dir().join(format!(
            "winios-auto-save-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("update-notifications.json");
        let (mut state, mut raw) = read(&path).unwrap();
        state.enabled = true;
        state.last_attempt = Some(123);
        assert!(record_version(&mut state, "2.0.0"));
        save(&path, &mut raw, &state).unwrap();
        let (restored, _) = read(&path).unwrap();
        assert!(restored.enabled);
        assert_eq!(restored.last_attempt, Some(123));
        assert_eq!(restored.notified_versions, vec!["2.0.0"]);
        std::fs::write(&path, br#"{"enabled":false}"#).unwrap();
        assert!(save(&path, &mut raw, &state).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"enabled":false}"#);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
