// Maprisma Control for COSMIC: a panel applet that starts and stops Maprisma.
// The Maprisma icon with a line under it: green while Maprisma runs, red while
// it runs as a recorded test session (start.sh), none while it is stopped.
// Left click opens the menu, right click starts or stops Maprisma directly.
//
// Whether Maprisma runs is read from /proc every two seconds, so it also sees
// a Maprisma started from the launcher or a terminal.

use maprisma_applet::*;
mod corners;
#[allow(dead_code)] // the shared copy; not every part is used here
mod look;
mod shot;
mod shot_files;
use shot::{Shot, ShotMsg};
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
const SHOT_FOLDER: &str = "Maprisma Control";
static AUTOSIZE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("maprisma-autosize"));

fn main() -> cosmic::iced::Result {
    if std::env::args().any(|arg| arg == "--window") {
        cosmic::app::run::<Control>(
            cosmic::app::Settings::default()
                .size(cosmic::iced::Size::new(420.0, 740.0)), true)
    } else {
        cosmic::applet::run::<Control>(false)
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

// A popup section heading: small, bold, upper case, dim (as in the other Maprisma applets).
fn section<'a>(title: String) -> Element<'a, Message> {
    container(text::caption_heading(title.to_uppercase()).class(theme::Text::Custom(dim_text)))
        .padding([8, 16, 4, 16])
        .into()
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
    shot: Shot,
    corners: String, // Settings › Corners (corners.rs)
    screenshot_key: bool,
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
    Language(usize),
    Shot(ShotMsg),
    ScreenshotKey,
    Corners(usize),
}

impl Control {
    fn pending(&self) -> Option<Pending> {
        self.pending.map(|(p, _)| p)
    }

    fn refresh(&mut self) {
        self.running = scan();
        settle_pending(&self.running, &mut self.pending);
    }

    // Settings › F12 saves a screenshot (on unless `screenshot_key=false`) and › Corners, read
    // again on every tick: another Control (window or applet) may have changed them.
    fn read_look(&mut self) -> Task<Message> {
        self.screenshot_key = i18n::setting("screenshot_key").as_deref() != Some("false");
        let corners = corners::normalize(&i18n::setting("corners").unwrap_or_default()).to_string();
        let mut restyle = corners != self.corners;
        self.corners = corners;
        corners::set_current(&self.corners);
        // The desktop's colours and corners (Maprisma Theme can change them any time);
        // look::refresh re-reads at most every 5 s.
        restyle |= look::refresh();
        if restyle && self.standalone {
            return corners::set_app_theme(&self.corners);
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
        let header = row![
            icon::from_name(ICON).size(32).icon(),
            column![
                text::title4(title),
                row![dot, dim(self.status_text())]
                    .spacing(8)
                    .align_y(Alignment::Center)
            ]
            .spacing(4),
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
        let mut actions = column![divider::horizontal::default()];
        if running {
            actions = actions
                .push(
                    menu_button(text::body(tr("Show / hide window")))
                        .on_press_maybe(idle.then_some(Message::ToggleWindow)),
                )
                .push(
                    menu_button(text::body(tr("Stop Maprisma")))
                        .on_press_maybe(idle.then_some(Message::Stop)),
                );
        } else {
            actions = actions
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
        actions = actions
            .push(divider::horizontal::default())
            .push(
                menu_button(text::body(tr("Open notes of the last test")))
                    .on_press_maybe(has_run.then_some(Message::OpenNotes)),
            )
            .push(menu_button(text::body(tr("Open the test folder"))).on_press(Message::OpenFolder));

        actions = actions.push(divider::horizontal::default()).push(section(tr("Language")));
        let preference = i18n::preference();
        for (index, label) in language_options().into_iter().enumerate() {
            let label = if i18n::CODES[index] == preference { format!("✓ {label}") } else { label };
            actions = actions.push(menu_button(text::body(label)).on_press(Message::Language(index)));
        }
        let check = |on: bool, label: String| if on { format!("✓ {label}") } else { label };
        actions = actions
            .push(divider::horizontal::default())
            .push(
                menu_button(text::body(check(self.screenshot_key, tr("F12 saves a screenshot"))))
                    .on_press(Message::ScreenshotKey),
            )
            .push(section(tr("Corners")));
        let desktop = corners::desktop_radius().round().to_string();
        let labels = [
            trf("Like the desktop ({0} px)", &[&desktop]),
            trf("Slightly rounded ({0} px)", &["6"]),
            trf("Rounded ({0} px)", &["12"]),
            tr("Square"),
        ];
        for (index, label) in labels.into_iter().enumerate() {
            actions = actions.push(
                menu_button(text::body(check(corners::CHOICES.iter().position(|c| *c == self.corners).unwrap_or(0) == index, label))).on_press(Message::Corners(index)),
            );
        }
        if let Some(error) = &self.error {
            actions = actions.push(padded_control(text::body(error.clone()).class(theme::Text::Custom(error_text))));
        }
        column![padded_control(info).padding([12, 16]), actions]
            .width(self.width())
            .padding([0, 0, 8, 0])
            .into()
    }

    // The popup's width; the window (--window) fills its width like the other Maprisma windows.
    fn width(&self) -> Length {
        if self.standalone { Length::Fill } else { Length::Fixed(POPUP_WIDTH) }
    }

    // The F12 note card above the menu while it is asked for.
    fn with_card<'a>(&'a self, id: window::Id, content: Element<'a, Message>) -> Element<'a, Message> {
        match self.shot.card(id, Message::Shot) {
            Some(card) => column![card, content].width(self.width()).into(),
            None => content,
        }
    }

    fn close_popup(&mut self) -> Task<Message> {
        match self.popup.take() {
            Some(popup) => {
                self.shot.closed(popup);
                destroy_popup(popup)
            }
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
        let mut control = Self {
            core,
            standalone,
            version: version(),
            shot: Shot::new(SHOT_FOLDER),
            ..Default::default()
        };
        control.refresh();
        let look = control.read_look();
        let task = if standalone {
            control.set_header_title(tr("Maprisma Control"));
            let id = control.core.main_window_id().unwrap();
            // The desktop's theme for the whole window (title bar too), see look.rs.
            Task::batch([control.set_window_title(tr("Maprisma Control"), id), corners::set_app_theme(&control.corners)])
        } else { Task::none() };
        (control, Task::batch([look, task]))
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        // Faster while starting or stopping, so the line changes right away.
        let every = if self.pending.is_some() {
            Duration::from_millis(400)
        } else {
            POLL
        };
        Subscription::batch(vec![
            rectangle_tracker_subscription(0).map(|update| Message::Rectangle(update.1)),
            cosmic::iced::time::every(every).map(|_| Message::Tick),
            Shot::subscription().map(Message::Shot),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Language(index) => {
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
            Message::Shot(message) => {
                let ours = match &message {
                    ShotMsg::Key(id) => self.shot_target(*id),
                    _ => true,
                };
                return self.shot.update(message, self.screenshot_key, ours, Message::Shot);
            }
            Message::ScreenshotKey => {
                let on = !self.screenshot_key;
                self.error = i18n::set_setting("screenshot_key", if on { "true" } else { "false" })
                    .err()
                    .map(|error| error.to_string());
                return self.read_look();
            }
            Message::Corners(index) => {
                let corners = corners::CHOICES.get(index).copied().unwrap_or("desktop");
                self.error = i18n::set_setting("corners", corners).err().map(|error| error.to_string());
                return self.read_look();
            }
            Message::Rectangle(update) => match update {
                RectangleUpdate::Rectangle(r) => self.rectangle = r.1,
                RectangleUpdate::Init(tracker) => self.tracker = Some(tracker),
            },
            Message::CloseRequested(id) => {
                self.shot.closed(id);
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
        if self.standalone {
            let id = self.core.main_window_id().unwrap_or(window::Id::RESERVED);
            return corners::apply_window(&self.corners, cosmic::widget::scrollable(self.with_card(id, self.menu())));
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
        if self.standalone {
            return corners::apply_window(&self.corners, cosmic::widget::scrollable(self.with_card(id, self.menu())));
        }
        corners::apply(&self.corners, self.core.applet.popup_container(self.with_card(id, self.menu())))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        // The window (--window) draws the theme's background; the applet stays transparent.
        if self.standalone { None } else { Some(cosmic::applet::style()) }
    }

}
