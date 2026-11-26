// Maprisma Control for COSMIC: a panel applet that starts and stops Maprisma.
// The Maprisma icon with a line under it: green while Maprisma runs, red while
// it runs as a recorded test session (start.sh), none while it is stopped.
// Left click opens the menu, right click starts or stops Maprisma directly.
//
// Whether Maprisma runs is read from /proc every two seconds, so it also sees
// a Maprisma started from the launcher or a terminal.

use maprisma_applet::*;
use maprisma_kit::{AppInfo, corners, look, shot, window_size};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::{menu_button, padded_control};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::destroy_popup;
use cosmic::iced::widget::container as iced_container;
use cosmic::iced::widget::text as iced_text;
use cosmic::iced::{Alignment, Background, Border, Color, Length, Rectangle, Subscription, window};
use cosmic::widget::rectangle_tracker::{
    RectangleTracker, RectangleUpdate, rectangle_tracker_subscription,
};
use cosmic::widget::{
    Id, autosize, button, column, container, divider, icon, mouse_area, row, space, text,
};
use cosmic::{ApplicationExt, Element, theme};

const APP_ID: &str = "io.github.ubruckhaus.CosmicMaprisma";
const ICON: &str = "com.maprisma.Maprisma";
/// The bar icon: a name of its own (Maprisma Manager can change it); its default is Maprisma's
/// coloured logo, so it is drawn in its own colours, not tinted like a symbolic icon.
const BAR_ICON: &str = "io.github.ubruckhaus.CosmicMaprisma-symbolic";
const POPUP_WIDTH: f32 = 340.0;
/// F12's folder in ~/Pictures and (as a slug) its file names.
const SHOT_APP: AppInfo = AppInfo::new(APP_ID, "Maprisma Control");
static AUTOSIZE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("maprisma-autosize"));
static NOTE_ID: LazyLock<Id> = LazyLock::new(Id::unique);

fn main() -> cosmic::iced::Result {
    if std::env::args().any(|arg| arg == "--window") {
        let (width, height) = window_start_size();
        cosmic::app::run::<Control>(
            cosmic::app::Settings::default().default_font(look::default_font())
                .size(cosmic::iced::Size::new(width, height)), true)
    } else {
        look::keep_fonts();
        cosmic::applet::run::<Control>(false)
    }
}

// The `--window` size: the one the user left it at (layout rule 5a), else 420 × 740.
fn window_start_size() -> window_size::Size {
    let saved = i18n::setting(window_size::KEY).and_then(|text| serde_json::from_str(&text).ok());
    window_size::parse(saved.as_ref()).unwrap_or((420.0, 740.0))
}

/// What Esc does (layout rule 5a): the innermost open thing first, then the window or popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscStep {
    /// Skip the screenshot's note (or close its error).
    Note,
    /// Close the open Language / Corners list.
    Menus,
    /// Leave the settings page.
    Back,
    /// Close the window (`--window`) or the popup.
    Close,
    Nothing,
}

/// `shown`: Esc came from the window or the open popup (not the panel button); `note`: the
/// screenshot card shows; `menus`: a list is open; `settings`: the settings page shows.
fn esc_step(shown: bool, note: bool, menus: bool, settings: bool) -> EscStep {
    if !shown {
        EscStep::Nothing
    } else if note {
        EscStep::Note
    } else if menus {
        EscStep::Menus
    } else if settings {
        EscStep::Back
    } else {
        EscStep::Close
    }
}

// ------------------------------------------------------------------------- processes

// ------------------------------------------------------------------------- widgets

fn dim_text(theme: &cosmic::Theme) -> iced_text::Style {
    let mut color = Color::from(theme.cosmic().on_bg_color());
    color.a = 0.6;
    iced_text::Style {
        color: Some(color),
        ..Default::default()
    }
}

fn dim<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a) -> Element<'a, Message> {
    text::caption(content)
        .class(theme::Text::Custom(dim_text))
        .into()
}

fn error_text(theme: &cosmic::Theme) -> iced_text::Style {
    iced_text::Style {
        color: Some(Color::from(theme.cosmic().destructive_color())),
        ..Default::default()
    }
}

fn fill(status: Option<Status>, radius: f32) -> theme::Container<'static> {
    theme::Container::custom(move |theme: &cosmic::Theme| iced_container::Style {
        background: Some(Background::Color(Status::color(status, theme))),
        border: Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    })
}

// Maprisma's state, in the theme's colours: running (green), a recorded test session (red),
// starting or stopping (yellow); stopped: a dim grey dot and no line.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Running,
    Recording,
    Pending,
}

impl Status {
    fn color(status: Option<Status>, theme: &cosmic::Theme) -> Color {
        let c = theme.cosmic();
        match status {
            Some(Status::Running) => c.success_color().into(),
            Some(Status::Recording) => c.destructive_color().into(),
            Some(Status::Pending) => c.warning_color().into(),
            None => {
                let mut grey = Color::from(c.on_bg_color());
                grey.a = 0.4;
                grey
            }
        }
    }
}

// ------------------------------------------------------------------------- applet

#[derive(Default)]
struct Control {
    core: Core,
    standalone: bool,
    error: Option<String>,
    popup: Option<window::Id>,
    rectangle: Rectangle,
    tracker: Option<RectangleTracker<u32>>,
    running: Option<Running>,
    pending: Option<(Pending, Instant)>,
    version: String,
    shot: shot::Phase,
    shot_window: Option<window::Id>,
    corners: String, // Settings › Corners (corners.rs)
    skin: String,    // Settings › App skin (skin.rs)
    screenshot_key: bool,
    resize: window_size::Debounce,
    /// The settings page (Language, F12, Corners, App skin) is shown instead of the menu.
    settings_open: bool,
    language_open: bool,
    corners_open: bool,
    skin_open: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Escape(window::Id),
    SaveSize,
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
    Language(usize),
    ShotKey(window::Id),
    ShotCaptured(window::Id, window::Screenshot),
    ShotSaved(window::Id, Result<shot::Saved, String>),
    NoteInput(String),
    NoteDone,
    NoteSaved(window::Id, Result<(), String>),
    ScreenshotKey,
    Corners(usize),
    Skin(String),
    ToggleSettings,
    ToggleLanguage,
    ToggleCorners,
    ToggleSkin,
}

impl Control {
    fn pending(&self) -> Option<Pending> {
        self.pending.map(|(p, _)| p)
    }

    fn refresh(&mut self) {
        self.running = scan();
        settle_pending(&self.running, &mut self.pending);
    }

    // Settings › F12 saves a screenshot (on unless `screenshot_key=false`), › Corners and › App
    // skin, read again on every tick: another Control (window or applet) may have changed them.
    fn read_look(&mut self) -> Task<Message> {
        self.screenshot_key = i18n::setting("screenshot_key").as_deref() != Some("false");
        let corners = corners::normalize(&i18n::setting("corners").unwrap_or_default()).to_string();
        let skin = skin::normalize_skin(&i18n::setting(skin::SKIN_KEY).unwrap_or_default()).to_string();
        let mut restyle = corners != self.corners || skin != self.skin;
        self.corners = corners;
        self.skin = skin;
        corners::set_current(&self.corners);
        // The desktop's colours and corners (Maprisma Theme can change them any time);
        // look::refresh re-reads at most every 5 s.
        restyle |= look::refresh();
        if restyle && self.standalone {
            return skin::apply_window_theme(&self.skin, &self.corners);
        }
        Task::none()
    }

    // F12 may take the open popup, or the window of `--window`; not the panel button.
    fn shot_target(&self, id: window::Id) -> bool {
        self.popup == Some(id) || (self.standalone && self.core.main_window_id() == Some(id))
    }

    fn status(&self) -> Option<Status> {
        if self.pending.is_some() {
            return Some(Status::Pending);
        }
        let running = self.running.as_ref()?;
        Some(if running.recording.is_some() {
            Status::Recording
        } else {
            Status::Running
        })
    }

    fn status_text(&self) -> String {
        status_text(&self.running, self.pending())
    }

    fn panel_content(&self) -> Element<'_, Message> {
        let size = self.core.applet.suggested_size(true).0;
        let logo = icon::from_name(BAR_ICON).size(size).symbolic(false).icon();
        let line: Element<Message> = match self.status() {
            Some(status) => container(space::horizontal())
                .width(Length::Fixed(size as f32))
                .height(Length::Fixed(2.0))
                .class(fill(Some(status), 1.0))
                .into(),
            None => space::vertical().height(Length::Fixed(2.0)).into(),
        };
        column![logo, line]
            .spacing(2)
            .align_x(Alignment::Center)
            .into()
    }

    fn menu(&self) -> Element<'_, Message> {
        let mut title = String::from("Maprisma");
        if !self.version.is_empty() {
            title += &format!(" {}", self.version);
        }
        let dot = container(space::horizontal())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .class(fill(self.status(), corners::now().pill.min(4.0)));
        let status_row = row![dot, dim(self.status_text())]
            .spacing(8)
            .align_y(Alignment::Center);
        let status_el: Element<Message> = if skin::skin_style(&self.skin).is_some() {
            container(status_row)
                .padding([2, 8])
                .class(skin::pill_style(&self.skin))
                .into()
        } else {
            status_row.into()
        };
        let header = row![
            icon::from_name(ICON).size(32).icon(),
            column![
                text::title4(title),
                status_el
            ]
            .spacing(4),
            space::horizontal().width(Length::Fill),
            // Settings at the header's right end, as in Maprisma Launcher.
            cosmic::widget::tooltip(
                button::icon(icon::from_name("emblem-system-symbolic").symbolic(true)).on_press(Message::ToggleSettings),
                text::body(tr("Settings")),
                cosmic::widget::tooltip::Position::Bottom,
            ),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let mut info = column![header].spacing(8);
        if let Some(dir) = self.running.as_ref().and_then(|r| r.recording.as_ref()) {
            info = info.push(dim(trf("Recording into {0}", &[&dir.display().to_string()])));
            info = info.push(dim(
                tr("F12 in Maprisma: a screenshot with a note. Stop or close it to finish."),
            ));
        }

        let idle = self.pending.is_none();
        let running = self.running.is_some();
        let has_run = last_run().is_some();
        let skinned = skin::skin_style(&self.skin).is_some();
        let mut primary_actions = column![];
        if running {
            primary_actions = primary_actions
                .push(
                    menu_button(text::body(tr("Show / hide window")))
                        .on_press_maybe(idle.then_some(Message::ToggleWindow)),
                )
                .push(
                    menu_button(text::body(tr("Stop Maprisma")))
                        .on_press_maybe(idle.then_some(Message::Stop)),
                );
        } else {
            primary_actions = primary_actions
                .push(
                    menu_button(text::body(tr("Start Maprisma")))
                        .on_press_maybe(idle.then_some(Message::Start)),
                )
                .push(
                    menu_button(text::body(tr("Start a recorded test session"))).on_press_maybe(
                        (idle && test_dir().join("start.sh").is_file())
                            .then_some(Message::StartRecorded),
                    ),
                );
        }
        let secondary_actions = column![
            menu_button(text::body(tr("Open notes of the last test")))
                .on_press_maybe(has_run.then_some(Message::OpenNotes)),
            menu_button(text::body(tr("Open the test folder"))).on_press(Message::OpenFolder),
        ];

        let mut content = if skinned {
            let header_card = container(info)
                .width(Length::Fill)
                .padding([12, 14])
                .class(skin::card_style(&self.skin));
            let primary_card = container(primary_actions)
                .width(Length::Fill)
                .padding([4, 4])
                .class(skin::sub_card_style(&self.skin));
            let secondary_card = container(secondary_actions)
                .width(Length::Fill)
                .padding([4, 4])
                .class(skin::sub_card_style(&self.skin));
            column![header_card, primary_card, secondary_card]
                .spacing(8)
                .padding([10, 12, 10, 12])
        } else {
            let actions = column![
                divider::horizontal::default(),
                primary_actions,
                divider::horizontal::default(),
                secondary_actions,
            ];
            column![padded_control(info).padding([12, 16]), actions]
                .padding([0, 0, 8, 0])
        };

        if let Some(error) = &self.error {
            content = content.push(padded_control(text::body(error.clone()).class(theme::Text::Custom(error_text))));
        }
        content
            .width(self.width())
            .into()
    }

    /// Settings (as Maprisma Launcher's page): ← Back and the title, then one row per setting,
    /// words at the left and the control at the right; a list opens under its row.
    fn settings_page(&self) -> Element<'_, Message> {
        let header = row![
            button::text(tr("Back")).leading_icon(icon::from_name("go-previous-symbolic").size(16)).on_press(Message::ToggleSettings),
            space::horizontal().width(Length::Fill),
            dim(tr("Settings").to_uppercase()),
        ]
        .align_y(Alignment::Center);
        let choice = |label: String, open: bool, message: Message| -> Element<'static, Message> {
            button::text(label)
                .trailing_icon(icon::from_name(if open { "pan-up-symbolic" } else { "pan-down-symbolic" }).size(16))
                .on_press(message)
                .into()
        };
        let setting = |words: Element<'static, Message>, control: Element<'static, Message>| {
            padded_control(row![container(words).width(Length::Fill), control].spacing(12).align_y(Alignment::Center))
        };
        let mut page = column![padded_control(header), divider::horizontal::default()];
        let languages = language_options();
        let preference = i18n::preference();
        let current = i18n::CODES.iter().position(|c| *c == preference).unwrap_or(0);
        page = page.push(setting(text::body(tr("Language")).into(), choice(languages[current].clone(), self.language_open, Message::ToggleLanguage)));
        if self.language_open {
            for (index, label) in languages.into_iter().enumerate() {
                let label = if index == current { format!("✓ {label}") } else { label };
                page = page.push(menu_button(text::body(label)).on_press(Message::Language(index)));
            }
        }
        page = page.push(setting(
            text::body(tr("F12 saves a screenshot")).into(),
            cosmic::widget::toggler(self.screenshot_key).on_toggle(|_| Message::ScreenshotKey).into(),
        ));
        let desktop = corners::desktop_radius().round().to_string();
        let labels = [
            trf("Like the desktop ({0} px)", &[&desktop]),
            trf("Slightly rounded ({0} px)", &["6"]),
            trf("Rounded ({0} px)", &["12"]),
            tr("Square"),
        ];
        let current = corners::CHOICES.iter().position(|c| *c == self.corners).unwrap_or(0);
        page = page.push(setting(text::body(tr("Corners")).into(), choice(labels[current].clone(), self.corners_open, Message::ToggleCorners)));
        if self.corners_open {
            for (index, label) in labels.into_iter().enumerate() {
                let label = if index == current { format!("✓ {label}") } else { label };
                page = page.push(menu_button(text::body(label)).on_press(Message::Corners(index)));
            }
        }
        page = page.push(setting(
            column![
                text::body(tr("App skin")),
                dim(tr("Visual accent for cards and borders; Native keeps your desktop theme untouched.")),
            ]
            .spacing(2)
            .into(),
            choice(skin::skin_label(&self.skin), self.skin_open, Message::ToggleSkin),
        ));
        if self.skin_open {
            for skin_id in skin::SKINS {
                let name = skin::skin_label(skin_id);
                let label = if self.skin == skin_id { format!("✓ {name}") } else { name };
                let row_inner: Element<Message> = if let Some(st) = skin::skin_style(skin_id) {
                    row![skin::swatch(st.accent, 10.0), text::body(label)]
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .into()
                } else {
                    text::body(label).into()
                };
                page = page.push(menu_button(row_inner).on_press(Message::Skin(skin_id.into())));
            }
        }
        if let Some(error) = &self.error {
            page = page.push(padded_control(text::body(error.clone()).class(theme::Text::Custom(error_text))));
        }
        page.width(self.width()).padding([0, 0, 8, 0]).into()
    }

    /// The menu, or the settings page while it is open.
    fn page(&self) -> Element<'_, Message> {
        if self.settings_open { self.settings_page() } else { self.menu() }
    }

    // The popup's width; the window (--window) fills its width like the other Maprisma windows.
    fn width(&self) -> Length {
        if self.standalone { Length::Fill } else { Length::Fixed(POPUP_WIDTH) }
    }

    fn shot_showing(&self, id: window::Id) -> bool {
        self.shot_window == Some(id) && self.shot.open()
    }

    fn finish_note(&mut self, keep: bool) -> Task<Message> {
        let win = self.shot_window;
        match std::mem::take(&mut self.shot) {
            shot::Phase::Note { path, text, .. } => {
                if keep && shot::note_text(&text).is_some() {
                    self.shot = shot::Phase::Noting;
                    let id = win.unwrap_or(window::Id::RESERVED);
                    return off_thread(
                        move || {
                            shot::save_note_for(&SHOT_APP, &path, &text)
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        },
                        move |result| Message::NoteSaved(id, result),
                    );
                }
                self.shot_window = None;
            }
            shot::Phase::Error(_) => self.shot_window = None,
            other => self.shot = other,
        }
        Task::none()
    }

    fn shot_closed(&mut self, id: window::Id, exiting: bool) -> Task<Message> {
        if self.shot_window != Some(id) {
            return Task::none();
        }
        match &self.shot {
            shot::Phase::Busy => {
                self.shot = shot::Phase::Idle;
                self.shot_window = None;
                Task::none()
            }
            shot::Phase::Note { .. } => {
                if exiting {
                    if let shot::Phase::Note { path, text, .. } = std::mem::take(&mut self.shot) {
                        let _ = shot::save_note_for(&SHOT_APP, &path, &text);
                    }
                    self.shot_window = None;
                    Task::none()
                } else {
                    self.finish_note(true)
                }
            }
            shot::Phase::Error(_) => {
                self.shot = shot::Phase::Idle;
                self.shot_window = None;
                Task::none()
            }
            shot::Phase::Idle | shot::Phase::Noting => Task::none(),
        }
    }

    // The F12 note card above the menu while it is asked for.
    fn with_card<'a>(&'a self, id: window::Id, content: Element<'a, Message>) -> Element<'a, Message> {
        if self.shot_window != Some(id) {
            return content;
        }
        let labels = shot::ShotLabels {
            saved_heading: tr("Screenshot saved"),
            saved_as_template: tr("Saved as {0}"),
            note_placeholder: tr("What is wrong? (optional)"),
            done_button: tr("Done"),
            save_error_template: tr("The screenshot could not be saved: {0}"),
            note_error_template: tr("The note could not be saved: {0}"),
        };
        let style = if skin::skin_style(&self.skin).is_some() {
            skin::card_style(&self.skin)
        } else {
            theme::Container::Card
        };
        match shot::note_overlay(
            &self.shot,
            NOTE_ID.clone(),
            &labels,
            style,
            Length::Fill,
            Message::NoteInput,
            Message::NoteDone,
        ) {
            Some(card) => column![card, content].width(self.width()).into(),
            None => content,
        }
    }

    // A resize that has settled (or, on `closing`, any waiting one) goes to settings.conf; not
    // while maximised, fullscreen or tiled (libcosmic's sharp corners).
    fn save_size(&mut self, closing: bool) {
        let sharp = self.core.window.sharp_corners;
        let ignore = || window_size::not_users_size(sharp);
        let size = if closing { self.resize.flush(ignore) } else { self.resize.due(Instant::now(), ignore) };
        if let Some(size) = size {
            self.error = i18n::set_setting(window_size::KEY, &window_size::to_value(size).to_string())
                .err()
                .map(|error| error.to_string());
        }
        // Hyprland: where the user left it, for the next open (a move sends no event, so on close).
        if closing && let Some(position) = window_size::current_position() {
            if let Err(error) = i18n::set_setting(window_size::POSITION_KEY, &window_size::position_value(position).to_string()) {
                self.error = Some(error.to_string());
            }
        }
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        self.language_open = false;
        self.corners_open = false;
        self.skin_open = false;
    }

    fn close_popup(&mut self) -> Task<Message> {
        match self.popup.take() {
            Some(popup) => {
                let shot_task = self.shot_closed(popup, false);
                self.close_settings();
                Task::batch([shot_task, destroy_popup(popup)])
            }
            None => Task::none(),
        }
    }
}

fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    wrap: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
) -> Task<Message> {
    cosmic::task::future(async move {
        let (tx, rx) = cosmic::iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        let result = rx
            .await
            .unwrap_or_else(|_| Err("the picture could not be written".into()));
        wrap(result)
    })
}

/// COSMIC `--window` with chosen corners: the app theme is a Custom copy, and libcosmic applies
/// system theme changes only to a System theme. So the new system theme ([base]) gets the chosen
/// corners again. None when nothing is to do: an applet popup (it themes per view), a non-native
/// skin (keeps its own palette), "desktop" corners (the System theme follows by itself) or a
/// desktop whose colours look.rs reads ([desktop_look]: KDE/Hyprland, re-read on the tick).
fn corner_theme_on(
    base: &cosmic::cosmic_theme::Theme,
    corners_setting: &str,
    skin_setting: &str,
    standalone: bool,
    desktop_look: bool,
) -> Option<cosmic::Theme> {
    if !standalone || desktop_look || skin::skin_style(skin_setting).is_some() || corners::normalize(corners_setting) == "desktop" {
        return None;
    }
    let mut custom = base.clone();
    custom.corner_radii = corners::radii(corners::sizes(corners_setting));
    let mut theme = cosmic::Theme::system(std::sync::Arc::new(base.clone()));
    theme.theme_type = cosmic::theme::ThemeType::Custom(std::sync::Arc::new(custom));
    Some(theme)
}

impl Control {
    fn follow_system_theme(&self, base: &cosmic::cosmic_theme::Theme) -> Task<Message> {
        let _ = corners::system_theme_update::<Message>(&self.corners, base, false);
        if skin::skin_style(&self.skin).is_some() {
            return if self.standalone {
                skin::apply_window_theme(&self.skin, &self.corners)
            } else {
                Task::none()
            };
        }
        match corner_theme_on(base, &self.corners, &self.skin, self.standalone, look::theme().is_some()) {
            Some(theme) => cosmic::command::set_theme(theme),
            None => Task::none(),
        }
    }
}

impl cosmic::Application for Control {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = bool;
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, standalone: bool) -> (Self, Task<Message>) {
        let mut core = core;
        look::solid(&mut core);
        let mut control = Self {
            core,
            standalone,
            version: version(),
            // Debug builds only: start on the settings page (for off-screen screenshots).
            settings_open: cfg!(debug_assertions) && std::env::var("MAPRISMA_SHOT_PAGE").as_deref() == Ok("settings"),
            ..Default::default()
        };
        control.refresh();
        if standalone {
            control.resize = window_size::Debounce::new(window_start_size());
            // Hyprland: back where the user left it (the tray rule first puts it under the bar).
            window_size::restore_position(window_size::parse_position(i18n::setting(window_size::POSITION_KEY).and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()).as_ref()), window_start_size());
        }
        let look = control.read_look();
        let task = if standalone {
            control.set_header_title(tr("Maprisma Control"));
            let id = control.core.main_window_id().unwrap();
            // The desktop's or skin's theme for the whole window (title bar too), see look.rs.
            Task::batch([
                control.set_window_title(tr("Maprisma Control"), id),
                skin::apply_window_theme(&control.skin, &control.corners),
            ])
        } else { Task::none() };
        (control, Task::batch([look, task]))
    }

    fn system_theme_update(&mut self, _keys: &[&'static str], new_theme: &cosmic::cosmic_theme::Theme) -> Task<Message> {
        self.follow_system_theme(new_theme)
    }

    fn system_theme_mode_update(&mut self, _keys: &[&'static str], mode: &cosmic::cosmic_theme::ThemeMode) -> Task<Message> {
        let system = if mode.is_dark { cosmic::theme::system_dark() } else { cosmic::theme::system_light() };
        self.follow_system_theme(system.cosmic())
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn on_window_resize(&mut self, id: window::Id, width: f32, height: f32) {
        if self.standalone && self.core.main_window_id() == Some(id) {
            self.resize.resized(width, height, Instant::now());
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Faster while starting or stopping, so the line changes right away.
        let every = if self.pending.is_some() {
            Duration::from_millis(400)
        } else {
            POLL
        };
        let save_size = if self.standalone && self.resize.waiting() {
            cosmic::iced::time::every(Duration::from_millis(100)).map(|_| Message::SaveSize)
        } else {
            Subscription::none()
        };
        Subscription::batch(vec![
            rectangle_tracker_subscription(0).map(|update| Message::Rectangle(update.1)),
            cosmic::iced::time::every(every).map(|_| Message::Tick),
            // F12 (no modifiers) in the popup or window; Esc closes the innermost thing (esc_step).
            cosmic::iced::event::listen_with(|event, status, id| match event {
                cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    ..
                }) => {
                    if shot::is_shot_key(&key, modifiers) {
                        Some(Message::ShotKey(id))
                    } else if status == cosmic::iced::event::Status::Ignored
                        && matches!(
                            key,
                            cosmic::iced::keyboard::Key::Named(
                                cosmic::iced::keyboard::key::Named::Escape
                            )
                        )
                    {
                        Some(Message::Escape(id))
                    } else {
                        None
                    }
                }
                _ => None,
            }),
            save_size,
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleSettings => {
                let open = !self.settings_open;
                self.close_settings();
                self.settings_open = open;
            }
            Message::ToggleLanguage => {
                self.language_open = !self.language_open;
                self.corners_open = false;
                self.skin_open = false;
            }
            Message::ToggleCorners => {
                self.corners_open = !self.corners_open;
                self.language_open = false;
                self.skin_open = false;
            }
            Message::ToggleSkin => {
                self.skin_open = !self.skin_open;
                self.language_open = false;
                self.corners_open = false;
            }
            Message::Language(index) => {
                self.language_open = false;
                self.error = i18n::set_language(index).err().map(|error|
                    trf("Could not save language: {0}", &[&error.to_string()]));
                if self.standalone {
                    self.set_header_title(tr("Maprisma Control"));
                    return self.set_window_title(tr("Maprisma Control"), self.core.main_window_id().unwrap());
                }
            }
            Message::Tick => {
                self.refresh();
                return self.read_look();
            }
            Message::Escape(id) => {
                let shown = self.shot_target(id);
                match esc_step(
                    shown,
                    self.shot_showing(id),
                    self.language_open || self.corners_open || self.skin_open,
                    self.settings_open,
                ) {
                    EscStep::Note => return self.finish_note(false),
                    EscStep::Menus => {
                        self.language_open = false;
                        self.corners_open = false;
                        self.skin_open = false;
                    }
                    EscStep::Back => self.close_settings(),
                    // Like the window's own close; the popup like a click on the icon.
                    EscStep::Close if self.standalone => {
                        let _ = self.update(Message::CloseRequested(id)); // only bookkeeping, no task
                        return cosmic::iced::exit();
                    }
                    EscStep::Close => return self.close_popup(),
                    EscStep::Nothing => {}
                }
            }
            Message::SaveSize => self.save_size(false),
            Message::ShotKey(id) => {
                if self.screenshot_key && self.shot_target(id) && self.shot == shot::Phase::Idle {
                    self.shot = shot::Phase::Busy;
                    self.shot_window = Some(id);
                    return window::screenshot(id)
                        .map(move |picture| cosmic::Action::App(Message::ShotCaptured(id, picture)));
                }
            }
            Message::ShotCaptured(id, picture) => {
                if self.shot == shot::Phase::Busy && self.shot_window == Some(id) {
                    let (width, height) = (picture.size.width, picture.size.height);
                    return off_thread(
                        move || shot::save_screenshot_for(&SHOT_APP, &picture.rgba, width, height),
                        move |result| Message::ShotSaved(id, result),
                    );
                }
            }
            Message::ShotSaved(id, result) => {
                if self.shot == shot::Phase::Busy && self.shot_window == Some(id) {
                    self.shot = match result {
                        Ok(saved) => shot::Phase::Note {
                            path: saved.path,
                            shown: saved.shown,
                            text: String::new(),
                        },
                        Err(reason) => shot::Phase::Error(
                            tr("The screenshot could not be saved: {0}").replace("{0}", &reason),
                        ),
                    };
                    if matches!(self.shot, shot::Phase::Note { .. }) {
                        return cosmic::widget::text_input::focus(NOTE_ID.clone());
                    }
                }
            }
            Message::NoteInput(value) => {
                if let shot::Phase::Note { text, .. } = &mut self.shot {
                    *text = value.chars().take(shot::NOTE_LIMIT).collect();
                }
            }
            Message::NoteDone => return self.finish_note(true),
            Message::NoteSaved(id, result) => match result {
                Ok(()) => {
                    if self.shot == shot::Phase::Noting {
                        self.shot = shot::Phase::Idle;
                        self.shot_window = None;
                    }
                }
                Err(reason) => {
                    self.shot_window = Some(id);
                    self.shot = shot::Phase::Error(
                        tr("The note could not be saved: {0}").replace("{0}", &reason),
                    );
                }
            },
            Message::ScreenshotKey => {
                let on = !self.screenshot_key;
                self.error = i18n::set_setting("screenshot_key", if on { "true" } else { "false" })
                    .err()
                    .map(|error| error.to_string());
                return self.read_look();
            }
            Message::Corners(index) => {
                self.corners_open = false;
                let corners = corners::CHOICES.get(index).copied().unwrap_or("desktop");
                self.error = i18n::set_setting("corners", corners).err().map(|error| error.to_string());
                return self.read_look();
            }
            Message::Skin(choice) => {
                self.skin_open = false;
                let choice = skin::normalize_skin(&choice);
                let value = (choice != "native").then_some(choice);
                self.error = i18n::set_optional_setting(skin::SKIN_KEY, value).err().map(|error| error.to_string());
                return self.read_look();
            }
            Message::Rectangle(update) => match update {
                RectangleUpdate::Rectangle(r) => self.rectangle = r.1,
                RectangleUpdate::Init(tracker) => self.tracker = Some(tracker),
            },
            Message::CloseRequested(id) => {
                let shot_task = self.shot_closed(id, self.standalone);
                if self.standalone {
                    self.save_size(true);
                }
                if self.popup == Some(id) {
                    self.popup = None;
                    self.close_settings();
                }
                return shot_task;
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
                        let mut settings = app.core.applet.get_popup_settings(
                            app.core.main_window_id().unwrap(),
                            id,
                            None,
                            None,
                            None,
                        );
                        let Rectangle {
                            x,
                            y,
                            width,
                            height,
                        } = app.rectangle;
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
                let action = match message {
                    Message::Start => Action::Start,
                    Message::StartRecorded => Action::StartRecorded,
                    Message::Stop => Action::Stop,
                    _ => Action::StartOrStop,
                };
                execute(action, &self.running, &mut self.pending);
                return self.close_popup();
            }
            Message::ToggleWindow => {
                execute(Action::ToggleWindow, &self.running, &mut self.pending);
                return self.close_popup();
            }
            Message::OpenFolder => {
                execute(Action::OpenFolder, &self.running, &mut self.pending);
                return self.close_popup();
            }
            Message::OpenNotes => {
                execute(Action::OpenNotes, &self.running, &mut self.pending);
                return self.close_popup();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        look::keep_fonts();
        if self.standalone {
            let id = self.core.main_window_id().unwrap_or(window::Id::RESERVED);
            return skin::apply_window(&self.skin, &self.corners, cosmic::widget::scrollable(self.with_card(id, self.page())));
        }
        let horizontal = matches!(
            self.core.applet.anchor,
            PanelAnchor::Top | PanelAnchor::Bottom
        );
        let padding = self.core.applet.suggested_padding(true).0;
        let button = button::custom(self.panel_content())
            .padding(if horizontal {
                [0, padding]
            } else {
                [padding, 0]
            })
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

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        look::keep_fonts();
        if self.standalone {
            return skin::apply_window(&self.skin, &self.corners, cosmic::widget::scrollable(self.with_card(id, self.page())));
        }
        skin::apply_popup(&self.skin, &self.corners, &self.core, self.with_card(id, self.page()))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // The window (--window) draws the theme's background; the applet stays transparent.
        if self.standalone { None } else { Some(cosmic::applet::style()) }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_closes_the_innermost_thing_first() {
        assert_eq!(esc_step(true, true, true, true), EscStep::Note);
        assert_eq!(esc_step(true, false, true, true), EscStep::Menus);
        assert_eq!(esc_step(true, false, false, true), EscStep::Back);
        assert_eq!(esc_step(true, false, false, false), EscStep::Close);
        // Not from the window or the open popup (e.g. the panel button): nothing.
        assert_eq!(esc_step(false, true, true, true), EscStep::Nothing);
        assert_eq!(esc_step(false, false, false, false), EscStep::Nothing);
    }

    #[test]
    fn custom_corners_follow_a_new_cosmic_theme() {
        let mut base = cosmic::cosmic_theme::Theme::dark_default();
        base.accent = cosmic::cosmic_theme::Component::colored_component(
            cosmic::cosmic_theme::palette::Srgba::new(0.9, 0.1, 0.1, 1.0),
            cosmic::cosmic_theme::palette::Srgba::new(0.0, 0.0, 0.0, 1.0),
            cosmic::cosmic_theme::palette::Srgba::new(0.0, 0.0, 0.0, 1.0),
            cosmic::cosmic_theme::palette::Srgba::new(0.0, 0.0, 0.0, 1.0),
            cosmic::cosmic_theme::palette::Srgba::new(0.0, 0.0, 0.0, 1.0),
        );
        let theme = corner_theme_on(&base, "rounded", "native", true, false).unwrap();
        let cosmic = theme.cosmic();
        assert_eq!(cosmic.corner_radii.radius_m, [12.0; 4]);
        // the new system colours, not the old custom copy
        assert_eq!(cosmic.accent.base, base.accent.base);
        assert!(matches!(theme.theme_type, cosmic::theme::ThemeType::Custom(_)));
        // nothing to do: applet popup, non-native skin, "desktop" corners, or KDE/Hyprland colours from look.rs
        assert!(corner_theme_on(&base, "rounded", "native", false, false).is_none());
        assert!(corner_theme_on(&base, "rounded", "synthwave", true, false).is_none());
        assert!(corner_theme_on(&base, "desktop", "native", true, false).is_none());
        assert!(corner_theme_on(&base, "square", "native", true, true).is_none());
    }

    #[test]
    fn window_size_as_a_setting_line() {
        let line = window_size::to_value((500.0, 800.0)).to_string();
        assert!(!line.contains('\n'));
        let back: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(window_size::parse(Some(&back)), Some((500.0, 800.0)));
    }
}
