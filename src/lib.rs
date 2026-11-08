use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const PROCESS: &str = "maprisma"; // /proc/<pid>/comm of the app (the core is maprisma-core)
pub const BINARY: &str = "/usr/bin/maprisma";
pub const DEFAULT_TEST_DIR: &str = "/mnt/Data/maprisma-test";
pub const POLL: Duration = Duration::from_secs(2);
pub const PENDING_TIMEOUT: Duration = Duration::from_secs(20); // "Starting…" / "Stopping…" at most this long
pub const KILL_AFTER: Duration = Duration::from_secs(8); // SIGTERM, then SIGKILL if it is still there
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Running {
    pub pids: Vec<i32>,
    pub since: i64,                 // unix time the oldest process started
    pub recording: Option<PathBuf>, // the run folder of a test session
}

pub fn test_dir() -> PathBuf {
    std::env::var_os("MAPRISMA_TEST_DIR").map_or_else(|| DEFAULT_TEST_DIR.into(), PathBuf::from)
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

// Our own processes named `name`, oldest first.
fn find(name: &str) -> Vec<(i32, i64)> {
    let me = uid();
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != me {
            continue;
        }
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        if comm.trim_end() == name {
            found.push((pid, meta.mtime()));
        }
    }
    found.sort_by_key(|(_, t)| *t);
    found
}

fn record_dir(pid: i32) -> Option<PathBuf> {
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    environ.split(|b| *b == 0).find_map(|var| {
        let value = var.strip_prefix(b"MAPRISMA_RECORD_PROTOCOL=")?;
        Some(
            Path::new(std::str::from_utf8(value).ok()?)
                .parent()?
                .to_path_buf(),
        )
    })
}

pub fn scan() -> Option<Running> {
    let found = find(PROCESS);
    let (_, since) = *found.first()?;
    let recording = found.iter().find_map(|(pid, _)| record_dir(*pid));
    Some(Running {
        pids: found.into_iter().map(|(pid, _)| pid).collect(),
        since,
        recording,
    })
}

/// Who process [pid] is: its start time (clock ticks since boot, /proc/pid/stat
/// field 22) while it is still our Maprisma (its name and owner), else None.
/// A PID the kernel gave to another process since has another start time.
fn identity(pid: i32) -> Option<u64> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    if comm.trim_end() != PROCESS {
        return None;
    }
    let owner = std::fs::metadata(format!("/proc/{pid}")).ok()?.uid();
    if owner != uid() {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // after "(comm)": state is field 3, starttime field 22
    stat.rsplit_once(") ")?.1.split_whitespace().nth(19)?.parse().ok()
}

// SIGTERM; a thread sends SIGKILL to what is still there after KILL_AFTER.
// Each signal only to the very process seen now (its start time checked
// again), never to one that took over its PID meanwhile.
pub fn stop(pids: Vec<i32>) {
    let seen: Vec<(i32, u64)> = pids.into_iter().filter_map(|pid| Some((pid, identity(pid)?))).collect();
    for (pid, _) in &seen {
        unsafe { libc::kill(*pid, libc::SIGTERM) };
    }
    std::thread::spawn(move || {
        std::thread::sleep(KILL_AFTER);
        for (pid, started) in seen {
            if identity(pid) == Some(started) {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    });
}

// Runs argv in its own process group (it outlives the applet); a thread reaps it.
pub fn spawn(argv: &[&str], dir: Option<&Path>) {
    let mut command = std::process::Command::new(argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

pub fn clock(unix: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = unix as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

// The newest run folder of the test sessions (named by date and time).
pub fn last_run() -> Option<PathBuf> {
    last_run_in(&test_dir())
}

fn last_run_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("notes.md").is_file())
        .max()
}

pub fn version() -> String {
    version_in(Path::new("/var/lib/pacman/local"))
}

fn version_in(dir: &Path) -> String {
    std::fs::read_dir(dir)
        .ok()
        .and_then(|dir| {
            dir.flatten().find_map(|e| {
                let name = e.file_name().into_string().ok()?;
                name.strip_prefix("maprisma-bin-").map(str::to_string)
            })
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    Starting,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Start,
    StartRecorded,
    Stop,
    StartOrStop,
    ToggleWindow,
    OpenFolder,
    OpenNotes,
    Activate,
}

pub fn execute(
    action: Action,
    running: &Option<Running>,
    pending: &mut Option<(Pending, Instant)>,
) {
    match action {
        Action::Activate => {
            if pending.is_none() {
                execute(
                    if running.is_some() {
                        Action::ToggleWindow
                    } else {
                        Action::Start
                    },
                    running,
                    pending,
                );
            }
        }
        Action::Start | Action::StartRecorded | Action::Stop | Action::StartOrStop => {
            if pending.is_some() {
                return;
            }
            // An explicit Start (perhaps from a menu shown before Maprisma was
            // started elsewhere) never stops it; only StartOrStop toggles.
            if matches!(action, Action::Start | Action::StartRecorded) && running.is_some() {
                return;
            }
            let start = action != Action::Stop && running.is_none();
            if start {
                if action == Action::StartRecorded {
                    let dir = test_dir();
                    spawn(
                        &["bash", &dir.join("start.sh").to_string_lossy()],
                        Some(&dir),
                    );
                } else {
                    spawn(&[BINARY], None);
                }
                *pending = Some((Pending::Starting, Instant::now()));
            } else if let Some(running) = running {
                stop(running.pids.clone());
                *pending = Some((Pending::Stopping, Instant::now()));
            }
        }
        Action::ToggleWindow => spawn(&[BINARY, "--toggle"], None),
        Action::OpenFolder => {
            let dir = running
                .as_ref()
                .and_then(|r| r.recording.clone())
                .unwrap_or_else(test_dir);
            spawn(&["xdg-open", &dir.to_string_lossy()], None);
        }
        Action::OpenNotes => {
            if let Some(run) = last_run() {
                spawn(&["xdg-open", &run.join("notes.md").to_string_lossy()], None);
            }
        }
    }
}

pub fn settle_pending(running: &Option<Running>, pending: &mut Option<(Pending, Instant)>) {
    if let Some((kind, at)) = *pending {
        if (kind == Pending::Starting) == running.is_some() || at.elapsed() > PENDING_TIMEOUT {
            *pending = None;
        }
    }
}
pub fn status_text(running: &Option<Running>, pending: Option<Pending>) -> String {
    match (pending, running) {
        (Some(Pending::Starting), _) => "Starting…".into(),
        (Some(Pending::Stopping), _) => "Stopping…".into(),
        (None, Some(r)) if r.recording.is_some() => {
            format!("Test session running since {}", clock(r.since))
        }
        (None, Some(r)) => format!("Running since {}", clock(r.since)),
        (None, None) => "Stopped".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_completion_and_timeout() {
        let running = Some(Running::default());
        for (kind, state, done) in [
            (Pending::Starting, &running, true),
            (Pending::Stopping, &None, true),
            (Pending::Starting, &None, false),
            (Pending::Stopping, &running, false),
        ] {
            let mut pending = Some((kind, Instant::now()));
            settle_pending(state, &mut pending);
            assert_eq!(pending.is_none(), done);
            let mut expired = Some((
                kind,
                Instant::now() - PENDING_TIMEOUT - Duration::from_secs(1),
            ));
            settle_pending(state, &mut expired);
            assert!(expired.is_none());
        }
    }
    #[test]
    fn statuses_and_pending_action_guard() {
        assert_eq!(status_text(&None, None), "Stopped");
        assert_eq!(status_text(&None, Some(Pending::Starting)), "Starting…");
        assert_eq!(status_text(&None, Some(Pending::Stopping)), "Stopping…");
        let r = Some(Running {
            since: 1,
            ..Default::default()
        });
        assert_eq!(status_text(&r, None), format!("Running since {}", clock(1)));
        let r = Some(Running {
            recording: Some("/tmp/run".into()),
            ..r.unwrap()
        });
        assert!(status_text(&r, None).starts_with("Test session running since "));
        let mut pending = Some((Pending::Stopping, Instant::now()));
        let original = pending;
        execute(Action::Start, &None, &mut pending);
        execute(Action::Activate, &None, &mut pending);
        assert_eq!(pending, original);
    }
    #[test]
    fn newest_notes_and_package_version() {
        let dir =
            std::env::temp_dir().join(format!("maprisma-control-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(last_run_in(&dir).is_none());
        assert_eq!(version_in(&dir), "");
        for name in [
            "2026-01-01",
            "2026-02-01",
            "2026-03-01",
            "maprisma-bin-2.4-1",
            "unrelated",
        ] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        for name in ["2026-01-01", "2026-02-01"] {
            std::fs::write(dir.join(name).join("notes.md"), "test").unwrap();
        }
        assert_eq!(last_run_in(&dir), Some(dir.join("2026-02-01")));
        assert_eq!(version_in(&dir), "2.4-1");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn proc_scan_finds_our_named_child_and_recording() {
        let mut child = std::process::Command::new("bash")
            .args(["-c", "printf maprisma > /proc/$$/comm; read -t 30"])
            .stdin(Stdio::piped())
            .env(
                "MAPRISMA_RECORD_PROTOCOL",
                "/tmp/maprisma-unit-test/protocol.jsonl",
            )
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        for _ in 0..100 {
            if identity(pid).is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let found = scan();
        let recording = record_dir(pid);
        stop(vec![pid]);
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(found.unwrap().pids.contains(&pid));
        assert_eq!(recording, Some(PathBuf::from("/tmp/maprisma-unit-test")));
        assert!(identity(pid).is_none());
    }
    #[test]
    fn stop_never_signals_a_process_that_is_not_our_maprisma() {
        // as when Maprisma ended and the kernel gave its PID to another program
        let mut other = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = other.id() as i32;
        stop(vec![pid]);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(other.try_wait().unwrap(), None, "sleep was signalled");
        other.kill().unwrap();
        other.wait().unwrap();
    }
}
