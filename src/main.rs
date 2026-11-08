// Maprisma Control for COSMIC: a panel applet that starts and stops Maprisma.
// The Maprisma icon with a line under it: green while Maprisma runs, red while
// it runs as a recorded test session (start.sh), none while it is stopped.
// Left click opens the menu, right click starts or stops Maprisma directly.
//
// Whether Maprisma runs is read from /proc every two seconds, so it also sees
// a Maprisma started from the launcher or a terminal.

use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::{menu_button, padded_control};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::destroy_popup;
use cosmic::iced::widget::container as iced_container;
use cosmic::iced::widget::text as iced_text;
use cosmic::iced::{Alignment, Background, Border, Color, Length, Rectangle, Subscription, window};
use cosmic::widget::rectangle_tracker::{RectangleTracker, RectangleUpdate, rectangle_tracker_subscription};
use cosmic::widget::{Id, autosize, button, column, container, divider, icon, mouse_area, row, space, text};
use cosmic::{Element, theme};

const APP_ID: &str = "io.github.ubruckhaus.CosmicMaprisma";
const ICON: &str = "com.maprisma.Maprisma";
const PROCESS: &str = "maprisma"; // /proc/<pid>/comm of the app (the core is maprisma-core)
const BINARY: &str = "/usr/bin/maprisma";
const DEFAULT_TEST_DIR: &str = "/mnt/Data/maprisma-test";
const POLL: Duration = Duration::from_secs(2);
const PENDING_TIMEOUT: Duration = Duration::from_secs(20); // "Starting…" / "Stopping…" at most this long
const KILL_AFTER: Duration = Duration::from_secs(8); // SIGTERM, then SIGKILL if it is still there
const POPUP_WIDTH: f32 = 340.0;
static AUTOSIZE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("maprisma-autosize"));

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Control>(())
}

// ------------------------------------------------------------------------- processes

#[derive(Debug, Clone, Default, PartialEq)]
struct Running {
    pids: Vec<i32>,
    since: i64,                // unix time the oldest process started
    recording: Option<PathBuf>, // the run folder of a test session
}

fn test_dir() -> PathBuf {
    std::env::var_os("MAPRISMA_TEST_DIR").map_or_else(|| DEFAULT_TEST_DIR.into(), PathBuf::from)
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

// Our own processes named `name`, oldest first.
fn find(name: &str) -> Vec<(i32, i64)> {
    let me = uid();
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return found };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != me {
            continue;
        }
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else { continue };
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
        Some(Path::new(std::str::from_utf8(value).ok()?).parent()?.to_path_buf())
    })
}

fn scan() -> Option<Running> {
    let found = find(PROCESS);
    let (_, since) = *found.first()?;
    let recording = found.iter().find_map(|(pid, _)| record_dir(*pid));
    Some(Running { pids: found.into_iter().map(|(pid, _)| pid).collect(), since, recording })
}

fn alive(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim_end() == PROCESS)
}

// SIGTERM; a thread sends SIGKILL to what is still there after KILL_AFTER.
fn stop(pids: Vec<i32>) {
    for pid in &pids {
        unsafe { libc::kill(*pid, libc::SIGTERM) };
    }
    std::thread::spawn(move || {
        std::thread::sleep(KILL_AFTER);
        for pid in pids {
            if alive(pid) {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    });
}

// Runs argv in its own process group (it outlives the applet); a thread reaps it.
fn spawn(argv: &[&str], dir: Option<&Path>) {
    let mut command = std::process::Command::new(argv[0]);
    command.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn clock(unix: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = unix as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

// The newest run folder of the test sessions (named by date and time).
fn last_run() -> Option<PathBuf> {
    std::fs::read_dir(test_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("notes.md").is_file())
        .max()
}

fn version() -> String {
    std::fs::read_dir("/var/lib/pacman/local")
        .ok()
        .and_then(|dir| {
            dir.flatten().find_map(|e| {
                let name = e.file_name().into_string().ok()?;
                name.strip_prefix("maprisma-bin-").map(str::to_string)
            })
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------------------- widgets

fn dim_text(theme: &cosmic::Theme) -> iced_text::Style {
    let mut color = Color::from(theme.cosmic().on_bg_color());
    color.a = 0.6;
    iced_text::Style { color: Some(color), ..Default::default() }
}

fn dim<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a) -> Element<'a, Message> {
    text::caption(content).class(theme::Text::Custom(dim_text)).into()
}

fn fill(color: Color, radius: f32) -> theme::Container<'static> {
    theme::Container::custom(move |_| iced_container::Style {
        background: Some(Background::Color(color)),
        border: Border { radius: radius.into(), ..Default::default() },
        ..Default::default()
    })
}

const GREEN: Color = Color::from_rgb(0x33 as f32 / 255.0, 0xd1 as f32 / 255.0, 0x7a as f32 / 255.0);
const RED: Color = Color::from_rgb(0xe0 as f32 / 255.0, 0x1b as f32 / 255.0, 0x24 as f32 / 255.0);
const AMBER: Color = Color::from_rgb(0xf6 as f32 / 255.0, 0xd3 as f32 / 255.0, 0x2d as f32 / 255.0);

// ------------------------------------------------------------------------- applet

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pending {
    Starting,
    Stopping,
}

#[derive(Default)]
struct Control {
    core: Core,
    popup: Option<window::Id>,
    rectangle: Rectangle,
    tracker: Option<RectangleTracker<u32>>,
    running: Option<Running>,
    pending: Option<(Pending, Instant)>,
    version: String,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    TogglePopup,
    CloseRequested(window::Id),
    Rectangle(RectangleUpdate<u32>),
    Start,
    StartRecorded,
    Stop,
    StartOrStop,
    ToggleWindow,
    OpenFolder,
    OpenNotes,
}

impl Control {
    fn pending(&self) -> Option<Pending> {
        self.pending.map(|(p, _)| p)
    }

    fn refresh(&mut self) {
        self.running = scan();
        if let Some((pending, at)) = self.pending {
            let done = match pending {
                Pending::Starting => self.running.is_some(),
                Pending::Stopping => self.running.is_none(),
            };
            if done || at.elapsed() > PENDING_TIMEOUT {
                self.pending = None;
            }
        }
    }

    fn status_color(&self) -> Option<Color> {
        if self.pending.is_some() {
            return Some(AMBER);
        }
        let running = self.running.as_ref()?;
        Some(if running.recording.is_some() { RED } else { GREEN })
    }

    fn status_text(&self) -> String {
        match (self.pending(), &self.running) {
            (Some(Pending::Starting), _) => "Starting…".into(),
            (Some(Pending::Stopping), _) => "Stopping…".into(),
            (None, Some(r)) if r.recording.is_some() => format!("Test session running since {}", clock(r.since)),
            (None, Some(r)) => format!("Running since {}", clock(r.since)),
            (None, None) => "Stopped".into(),
        }
    }

    fn panel_content(&self) -> Element<'_, Message> {
        let size = self.core.applet.suggested_size(true).0;
        let logo = icon::from_name(ICON).size(size).icon();
        let line: Element<Message> = match self.status_color() {
            Some(color) => container(space::horizontal())
                .width(Length::Fixed(size as f32))
                .height(Length::Fixed(2.0))
                .class(fill(color, 1.0))
                .into(),
            None => space::vertical().height(Length::Fixed(2.0)).into(),
        };
        column![logo, line].spacing(2).align_x(Alignment::Center).into()
    }

    fn menu(&self) -> Element<'_, Message> {
        let mut title = String::from("Maprisma");
        if !self.version.is_empty() {
            title += &format!(" {}", self.version);
        }
        let dot = container(space::horizontal())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .class(fill(self.status_color().unwrap_or(Color::from_rgba(0.5, 0.5, 0.5, 0.5)), 4.0));
        let header = row![
            icon::from_name(ICON).size(32).icon(),
            column![text::title4(title), row![dot, dim(self.status_text())].spacing(6).align_y(Alignment::Center)]
                .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let mut info = column![header].spacing(6);
        if let Some(dir) = self.running.as_ref().and_then(|r| r.recording.as_ref()) {
            info = info.push(dim(format!("Recording into {}", dir.display())));
            info = info.push(dim("F12 in Maprisma: a screenshot with a note. Stop or close it to finish."));
        }

        let idle = self.pending.is_none();
        let running = self.running.is_some();
        let has_run = last_run().is_some();
        let mut actions = column![divider::horizontal::default()];
        if running {
            actions = actions
                .push(menu_button(text::body("Show / hide window")).on_press_maybe(idle.then_some(Message::ToggleWindow)))
                .push(menu_button(text::body("Stop Maprisma")).on_press_maybe(idle.then_some(Message::Stop)));
        } else {
            actions = actions
                .push(menu_button(text::body("Start Maprisma")).on_press_maybe(idle.then_some(Message::Start)))
                .push(
                    menu_button(text::body("Start a recorded test session"))
                        .on_press_maybe((idle && test_dir().join("start.sh").is_file()).then_some(Message::StartRecorded)),
                );
        }
        actions = actions
            .push(divider::horizontal::default())
            .push(menu_button(text::body("Open notes of the last test")).on_press_maybe(has_run.then_some(Message::OpenNotes)))
            .push(menu_button(text::body("Open the test folder")).on_press(Message::OpenFolder));

        column![padded_control(info).padding([12, 16]), actions]
            .width(Length::Fixed(POPUP_WIDTH))
            .padding([0, 0, 8, 0])
            .into()
    }

    fn close_popup(&mut self) -> Task<Message> {
        match self.popup.take() {
            Some(popup) => destroy_popup(popup),
            None => Task::none(),
        }
    }
}

impl cosmic::Application for Control {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        let mut control = Self { core, version: version(), ..Default::default() };
        control.refresh();
        (control, Task::none())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        // Faster while starting or stopping, so the line changes right away.
        let every = if self.pending.is_some() { Duration::from_millis(400) } else { POLL };
        Subscription::batch(vec![
            rectangle_tracker_subscription(0).map(|update| Message::Rectangle(update.1)),
            cosmic::iced::time::every(every).map(|_| Message::Tick),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.refresh(),
            Message::Rectangle(update) => match update {
                RectangleUpdate::Rectangle(r) => self.rectangle = r.1,
                RectangleUpdate::Init(tracker) => self.tracker = Some(tracker),
            },
            Message::CloseRequested(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::TogglePopup => {
                if self.popup.is_some() {
                    return self.close_popup();
                }
                self.refresh();
                self.version = version();
                return cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut Self| {
                        let id = window::Id::unique();
                        app.popup = Some(id);
                        let mut settings =
                            app.core.applet.get_popup_settings(app.core.main_window_id().unwrap(), id, None, None, None);
                        let Rectangle { x, y, width, height } = app.rectangle;
                        settings.positioner.anchor_rect = Rectangle::<i32> {
                            x: x.max(1.0) as i32,
                            y: y.max(1.0) as i32,
                            width: width.max(1.0) as i32,
                            height: height.max(1.0) as i32,
                        };
                        settings.positioner.size = None;
                        settings
                    },
                    None,
                ));
            }
            Message::Start | Message::StartRecorded | Message::Stop | Message::StartOrStop => {
                self.refresh();
                if self.pending.is_some() {
                    return Task::none();
                }
                let start = match message {
                    Message::Start | Message::StartRecorded => self.running.is_none(),
                    Message::Stop => false,
                    _ => self.running.is_none(),
                };
                if start {
                    if matches!(message, Message::StartRecorded) {
                        let dir = test_dir();
                        spawn(&["bash", &dir.join("start.sh").to_string_lossy()], Some(&dir));
                    } else {
                        spawn(&[BINARY], None);
                    }
                    self.pending = Some((Pending::Starting, Instant::now()));
                } else if let Some(running) = &self.running {
                    stop(running.pids.clone());
                    self.pending = Some((Pending::Stopping, Instant::now()));
                }
                return self.close_popup();
            }
            Message::ToggleWindow => {
                spawn(&[BINARY, "--toggle"], None);
                return self.close_popup();
            }
            Message::OpenFolder => {
                let dir = self.running.as_ref().and_then(|r| r.recording.clone()).unwrap_or_else(test_dir);
                spawn(&["xdg-open", &dir.to_string_lossy()], None);
                return self.close_popup();
            }
            Message::OpenNotes => {
                if let Some(run) = last_run() {
                    spawn(&["xdg-open", &run.join("notes.md").to_string_lossy()], None);
                }
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let horizontal = matches!(self.core.applet.anchor, PanelAnchor::Top | PanelAnchor::Bottom);
        let padding = self.core.applet.suggested_padding(true).0;
        let button = button::custom(self.panel_content())
            .padding(if horizontal { [0, padding] } else { [padding, 0] })
            .on_press_down(Message::TogglePopup)
            .class(theme::Button::AppletIcon);
        // Right click: start or stop at once.
        let button = mouse_area(button).on_right_press(Message::StartOrStop);
        let content: Element<Message> = match self.tracker.as_ref() {
            Some(tracker) => tracker.container(0, button).ignore_bounds(true).into(),
            None => button.into(),
        };
        autosize::autosize(content, AUTOSIZE_ID.clone()).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        self.core.applet.popup_container(self.menu()).into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
