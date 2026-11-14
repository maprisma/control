pub mod i18n;
pub use i18n::{tr, trf, language_options};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

pub const PROCESS: &str = "maprisma"; // /proc/<pid>/comm of the app (the core is maprisma-core)
pub const BINARY: &str = "/usr/bin/maprisma";
pub const DEFAULT_TEST_DIR: &str = "/mnt/Data/maprisma-test";

/// A discovered program and the fixed arguments required by its packaging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub path: PathBuf,
    pub mode: LaunchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Bundle,
    AppImage,
    AppImageExtract,
}

impl LaunchSpec {
    fn bundle(path: PathBuf) -> Self {
        Self {
            path,
            mode: LaunchMode::Bundle,
        }
    }

    fn args(&self, toggle: bool) -> Vec<&'static str> {
        let mut args = Vec::new();
        if self.mode == LaunchMode::AppImageExtract {
            args.push("--appimage-extract-and-run");
        }
        if toggle {
            args.push("--toggle");
        }
        args
    }

    fn launch(&self, toggle: bool) {
        let path = self.path.to_string_lossy();
        let mut argv = vec![path.as_ref()];
        argv.extend(self.args(toggle));
        spawn(&argv, None);
    }
}

pub fn binary() -> LaunchSpec {
    discover(
        std::env::var_os("HOME").as_deref().map(Path::new),
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("XDG_STATE_HOME").as_deref().map(Path::new),
        uid(),
    )
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0)
}

fn discover(
    home: Option<&Path>,
    path: Option<&std::ffi::OsStr>,
    state: Option<&Path>,
    me: u32,
) -> LaunchSpec {
    if let Some(local) = home
        .map(|h| h.join(".local/bin/maprisma"))
        .filter(|p| executable(p))
    {
        return LaunchSpec::bundle(local);
    }
    if let Some(path) = path {
        for dir in std::env::split_paths(path).filter(|p| p.is_absolute()) {
            let candidate = dir.join("maprisma");
            if executable(&candidate) {
                return LaunchSpec::bundle(candidate);
            }
        }
    }
    let state = state
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|h| h.join(".local/state")));
    if let Some(spec) = state.and_then(|s| launch_record(&s.join("maprisma/launch.json"), me)) {
        return spec;
    }
    LaunchSpec::bundle(BINARY.into())
}

fn private_owner(owner: u32, mode: u32, me: u32) -> bool {
    owner == me && mode & 0o022 == 0
}

fn safe_exec_owner(owner: u32, mode: u32, me: u32) -> bool {
    (owner == me || owner == 0) && mode & 0o022 == 0
}

fn safe_ancestor(owner: u32, mode: u32) -> bool {
    mode & 0o022 == 0 || (owner == 0 && mode & 0o1000 != 0)
}

fn safe_exec(path: &Path, me: u32) -> Option<()> {
    if !path.is_absolute() {
        return None;
    }
    let direct = std::fs::symlink_metadata(path).ok()?;
    let resolved = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&resolved).ok()?;
    if !direct.is_file()
        || !meta.is_file()
        || direct.dev() != meta.dev()
        || direct.ino() != meta.ino()
        || meta.mode() & 0o111 == 0
        || !safe_exec_owner(meta.uid(), meta.mode(), me)
    {
        return None;
    }
    // Check both the supplied route (including symlink parents) and its resolved route.
    for route in [path, resolved.as_path()] {
        for ancestor in route.ancestors().skip(1) {
            let meta = std::fs::metadata(ancestor).ok()?;
            if !meta.is_dir() || !safe_ancestor(meta.uid(), meta.mode()) {
                return None;
            }
        }
    }
    Some(())
}

fn launch_record(path: &Path, me: u32) -> Option<LaunchSpec> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let folder = std::fs::metadata(path.parent()?).ok()?;
    if !folder.is_dir() || !private_owner(folder.uid(), folder.mode(), me) {
        return None;
    }
    // O_NOFOLLOW and the identity check also protect the gap between lstat and open.
    let direct = std::fs::symlink_metadata(path).ok()?;
    if !direct.is_file() {
        return None;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file()
        || meta.dev() != direct.dev()
        || meta.ino() != direct.ino()
        || !private_owner(meta.uid(), meta.mode(), me)
        || meta.len() > 4096
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let exec = json.get("exec")?.as_str()?;
    if exec.contains('\0') {
        return None;
    }
    let mode = match json.get("mode")?.as_str()? {
        "bundle" => LaunchMode::Bundle,
        "appimage" => LaunchMode::AppImage,
        "appimage-extract" => LaunchMode::AppImageExtract,
        _ => return None,
    };
    let path = PathBuf::from(exec);
    safe_exec(&path, me)?;
    Some(LaunchSpec { path, mode })
}
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
    Language(usize),
    /// The tray (KDE, Hyprland): Control's own window (`maprisma-applet --window`), which has
    /// the settings the tray menu lacks (Corners, F12).
    OpenControl,
}

/// The Control window's program: next to the tray's own (both are installed together), else
/// from PATH.
pub fn control_binary(tray: Option<&Path>) -> PathBuf {
    tray.and_then(Path::parent)
        .map(|dir| dir.join("maprisma-applet"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("maprisma-applet"))
}

pub fn execute(
    action: Action,
    running: &Option<Running>,
    pending: &mut Option<(Pending, Instant)>,
) {
    match action {
        Action::Language(index) => { if let Err(error) = i18n::set_language(index) { eprintln!("{}", trf("Could not save language: {0}", &[&error.to_string()])); } },
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
                let bin = binary();
                if action == Action::StartRecorded {
                    let dir = test_dir();
                    spawn(
                        &[
                            "bash",
                            &dir.join("start.sh").to_string_lossy(),
                            &bin.path.to_string_lossy(),
                        ],
                        Some(&dir),
                    );
                } else {
                    bin.launch(false);
                }
                *pending = Some((Pending::Starting, Instant::now()));
            } else if let Some(running) = running {
                stop(running.pids.clone());
                *pending = Some((Pending::Stopping, Instant::now()));
            }
        }
        Action::ToggleWindow => {
            let bin = binary();
            bin.launch(true);
        }
        Action::OpenFolder => {
            let dir = running
                .as_ref()
                .and_then(|r| r.recording.clone())
                .unwrap_or_else(test_dir);
            spawn(&["xdg-open", &dir.to_string_lossy()], None);
        }
        Action::OpenControl => {
            let exe = std::env::current_exe().ok();
            spawn(&[&control_binary(exe.as_deref()).to_string_lossy(), "--window"], None);
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
        (Some(Pending::Starting), _) => tr("Starting…"),
        (Some(Pending::Stopping), _) => tr("Stopping…"),
        (None, Some(r)) if r.recording.is_some() => {
            trf("Test session running since {0}", &[&clock(r.since)])
        }
        (None, Some(r)) => trf("Running since {0}", &[&clock(r.since)]),
        (None, None) => tr("Stopped"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct LaunchFixture(PathBuf);
    impl LaunchFixture {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            // The sandbox maps /tmp's root owner to nobody. Keep secure-exec
            // fixtures under target instead: the production policy must not be relaxed.
            let base = std::env::current_dir().unwrap().join("target/launch-tests");
            std::fs::create_dir_all(&base).unwrap();
            let dir = base.join(format!(
                "maprisma-launch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&dir).unwrap();
            chmod(&dir, 0o700);
            Self(dir)
        }
        fn program(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            chmod(&path, 0o700);
            path
        }
        fn record(&self, exec: &Path, mode: &str) -> PathBuf {
            let path = self.0.join("state/maprisma/launch.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            chmod(path.parent().unwrap(), 0o700);
            std::fs::write(
                &path,
                serde_json::json!({"exec":exec,"mode":mode}).to_string(),
            )
            .unwrap();
            chmod(&path, 0o600);
            path
        }
    }
    impl Drop for LaunchFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn chmod(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn launch_owner_and_permission_policies() {
        assert!(private_owner(123, 0o600, 123));
        assert!(!private_owner(0, 0o600, 123));
        assert!(!private_owner(124, 0o600, 123));
        for bit in [0o020, 0o002] {
            assert!(!private_owner(123, 0o600 | bit, 123));
            assert!(!safe_exec_owner(123, 0o700 | bit, 123));
            assert!(!safe_exec_owner(0, 0o700 | bit, 123));
            assert!(!safe_ancestor(0, 0o755 | bit));
            assert!(!safe_ancestor(123, 0o1755 | bit));
            assert!(safe_ancestor(0, 0o1755 | bit));
        }
        assert!(safe_exec_owner(0, 0o755, 123));
        assert!(safe_exec_owner(123, 0o700, 123));
        assert!(!safe_exec_owner(124, 0o700, 123));
        assert!(safe_ancestor(123, 0o755));
    }

    #[test]
    fn record_modes_and_fixed_launch_arguments() {
        let f = LaunchFixture::new();
        let exec = f.program("App Image ; literal");
        for (name, mode, first) in [
            ("bundle", LaunchMode::Bundle, vec![]),
            ("appimage", LaunchMode::AppImage, vec![]),
            (
                "appimage-extract",
                LaunchMode::AppImageExtract,
                vec!["--appimage-extract-and-run"],
            ),
        ] {
            let spec = launch_record(&f.record(&exec, name), uid()).unwrap();
            assert_eq!(spec.path, exec);
            assert_eq!(spec.mode, mode);
            assert_eq!(spec.args(false), first);
            let mut toggle = first;
            toggle.push("--toggle");
            assert_eq!(spec.args(true), toggle);
        }
    }

    #[test]
    fn record_json_and_size_validation() {
        let f = LaunchFixture::new();
        let exec = f.program("program");
        let record = f.record(&exec, "bundle");
        for invalid in [
            "{",
            "[]",
            "null",
            "{}",
            r#"{"exec":5,"mode":"bundle"}"#,
            r#"{"exec":"/bin/true","mode":5}"#,
            r#"{"exec":"relative","mode":"bundle"}"#,
            r#"{"exec":"/bad\u0000path","mode":"bundle"}"#,
        ] {
            std::fs::write(&record, invalid).unwrap();
            assert!(launch_record(&record, uid()).is_none(), "{invalid}");
        }
        assert!(launch_record(&f.record(&exec, "unknown"), uid()).is_none());
        f.record(&exec, "bundle");
        let mut bytes = std::fs::read(&record).unwrap();
        bytes.resize(4096, b' ');
        std::fs::write(&record, &bytes).unwrap();
        assert!(launch_record(&record, uid()).is_some());
        bytes.push(b' ');
        std::fs::write(&record, bytes).unwrap();
        assert!(launch_record(&record, uid()).is_none());
    }

    #[test]
    fn record_file_and_folder_validation() {
        use std::os::unix::fs::symlink;
        let f = LaunchFixture::new();
        let exec = f.program("program");
        let record = f.record(&exec, "bundle");
        assert!(launch_record(&record, uid().wrapping_add(1)).is_none());
        for mode in [0o620, 0o602] {
            chmod(&record, mode);
            assert!(launch_record(&record, uid()).is_none());
        }
        chmod(&record, 0o600);
        for mode in [0o720, 0o702] {
            chmod(record.parent().unwrap(), mode);
            assert!(launch_record(&record, uid()).is_none());
        }
        chmod(record.parent().unwrap(), 0o700);
        let saved = record.with_extension("saved");
        std::fs::rename(&record, &saved).unwrap();
        symlink(&saved, &record).unwrap();
        assert!(launch_record(&record, uid()).is_none());
        std::fs::remove_file(&record).unwrap();
        std::fs::create_dir(&record).unwrap();
        assert!(launch_record(&record, uid()).is_none());
        assert!(launch_record(&exec.join("launch.json"), uid()).is_none());
    }

    #[test]
    fn executable_and_ancestor_validation() {
        use std::os::unix::fs::symlink;
        let f = LaunchFixture::new();
        let exec = f.program("nested/program");
        assert!(safe_exec(&exec, uid()).is_some());
        for mode in [0o600, 0o720, 0o702] {
            chmod(&exec, mode);
            assert!(safe_exec(&exec, uid()).is_none());
        }
        chmod(&exec, 0o700);
        assert!(safe_exec(&f.0, uid()).is_none());
        assert!(safe_exec(&f.0.join("missing"), uid()).is_none());
        assert!(safe_exec(Path::new("relative"), uid()).is_none());
        let link = f.0.join("link");
        symlink(&exec, &link).unwrap();
        assert!(safe_exec(&link, uid()).is_none());
        for mode in [0o720, 0o702] {
            chmod(exec.parent().unwrap(), mode);
            assert!(safe_exec(&exec, uid()).is_none());
        }
        chmod(exec.parent().unwrap(), 0o700);
        // A symlinked ancestor is allowed, but its resolved directories must be safe.
        let alias = f.0.join("alias");
        symlink(exec.parent().unwrap(), &alias).unwrap();
        assert!(safe_exec(&alias.join("program"), uid()).is_some());
        chmod(exec.parent().unwrap(), 0o722);
        assert!(safe_exec(&alias.join("program"), uid()).is_none());
    }

    #[test]
    fn discovery_precedence_and_absolute_path_entries() {
        let f = LaunchFixture::new();
        let exec = f.program("portable");
        f.record(&exec, "appimage-extract");
        let state = f.0.join("state");
        let discover_here =
            |path: Option<&std::ffi::OsStr>| discover(Some(&f.0), path, Some(&state), uid());
        assert_eq!(discover_here(None).path, exec);
        assert_eq!(
            discover_here(Some(std::ffi::OsStr::new(":.:relative"))).path,
            exec
        );
        let path_program = f.program("path/maprisma");
        let other = f.program("other/maprisma");
        let path = std::env::join_paths([path_program.parent().unwrap(), other.parent().unwrap()])
            .unwrap();
        assert_eq!(discover_here(Some(&path)).path, path_program);
        let local = f.program(".local/bin/maprisma");
        assert_eq!(discover_here(Some(&path)).path, local);
        chmod(&local, 0o600);
        assert_eq!(discover_here(Some(&path)).path, path_program);
        chmod(&path_program, 0o600);
        assert_eq!(discover_here(Some(&path)).path, other);
        chmod(&other, 0o600);
        assert_eq!(discover_here(Some(&path)).mode, LaunchMode::AppImageExtract);
        std::fs::write(state.join("maprisma/launch.json"), "invalid").unwrap();
        assert_eq!(
            discover_here(Some(&path)),
            LaunchSpec::bundle(BINARY.into())
        );
        assert_eq!(
            discover(None, None, None, uid()).path,
            PathBuf::from(BINARY)
        );
        // Default state location; relative/empty XDG_STATE_HOME falls back to it.
        f.record(&exec, "bundle");
        std::fs::create_dir_all(f.0.join(".local")).unwrap();
        std::fs::rename(&state, f.0.join(".local/state")).unwrap();
        for state in [None, Some(Path::new("relative")), Some(Path::new(""))] {
            assert_eq!(discover(Some(&f.0), None, state, uid()).path, exec);
        }
    }

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
        assert_eq!(status_text(&None, None), tr("Stopped"));
        assert_eq!(status_text(&None, Some(Pending::Starting)), tr("Starting…"));
        assert_eq!(status_text(&None, Some(Pending::Stopping)), tr("Stopping…"));
        let r = Some(Running {
            since: 1,
            ..Default::default()
        });
        assert_eq!(status_text(&r, None), trf("Running since {0}", &[&clock(1)]));
        let r = Some(Running {
            recording: Some("/tmp/run".into()),
            ..r.unwrap()
        });
        assert_eq!(status_text(&r, None), trf("Test session running since {0}", &[&clock(1)]));
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
    fn control_binary_next_to_the_tray_else_path() {
        let dir = std::env::temp_dir().join(format!("maprisma-control-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tray = dir.join("maprisma-control-tray");
        assert_eq!(control_binary(None), PathBuf::from("maprisma-applet"));
        assert_eq!(control_binary(Some(&tray)), PathBuf::from("maprisma-applet"));
        std::fs::write(dir.join("maprisma-applet"), "").unwrap();
        assert_eq!(control_binary(Some(&tray)), dir.join("maprisma-applet"));
        std::fs::remove_dir_all(&dir).unwrap();
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
