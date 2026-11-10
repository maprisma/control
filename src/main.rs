// Maprisma Control for COSMIC: a panel applet that starts and stops Maprisma.
// The Maprisma icon with a line under it: green while Maprisma runs, red while
// it runs as a recorded test session (start.sh), none while it is stopped.
// Left click opens the menu, right click starts or stops Maprisma directly.
//
// Whether Maprisma runs is read from /proc every two seconds, so it also sees
// a Maprisma started from the launcher or a terminal.

use maprisma_applet::*;
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

fn fill(color: Color, radius: f32) -> theme::Container<'static> {
    theme::Container::custom(move |_| iced_container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    })
}

const GREEN: Color = Color::from_rgb(
    0x33 as f32 / 255.0,
    0xd1 as f32 / 255.0,
    0x7a as f32 / 255.0,
);
const RED: Color = Color::from_rgb(
    0xe0 as f32 / 255.0,
    0x1b as f32 / 255.0,
    0x24 as f32 / 255.0,
);
const AMBER: Color = Color::from_rgb(
    0xf6 as f32 / 255.0,
    0xd3 as f32 / 255.0,
    0x2d as f32 / 255.0,
);

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
}

impl Control {
    fn pending(&self) -> Option<Pending> {
        self.pending.map(|(p, _)| p)
    }

    fn refresh(&mut self) {
        self.running = scan();
        settle_pending(&self.running, &mut self.pending);
    }

    fn status_color(&self) -> Option<Color> {
        if self.pending.is_some() {
            return Some(AMBER);
        }
        let running = self.running.as_ref()?;
        Some(if running.recording.is_some() {
            RED
        } else {
            GREEN
        })
    }

    fn status_text(&self) -> String {
        status_text(&self.running, self.pending())
    }

    fn panel_content(&self) -> Element<'_, Message> {
        let size = self.core.applet.suggested_size(true).0;
        let logo = icon::from_name(BAR_ICON).size(size).symbolic(false).icon();
        let line: Element<Message> = match self.status_color() {
            Some(color) => container(space::horizontal())
                .width(Length::Fixed(size as f32))
                .height(Length::Fixed(2.0))
                .class(fill(color, 1.0))
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
            .class(fill(
                self.status_color()
                    .unwrap_or(Color::from_rgba(0.5, 0.5, 0.5, 0.5)),
                4.0,
            ));
        let header = row![
            icon::from_name(ICON).size(32).icon(),
            column![
                text::title4(title),
                row![dot, dim(self.status_text())]
                    .spacing(6)
                    .align_y(Alignment::Center)
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let mut info = column![header].spacing(6);
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

        actions = actions.push(divider::horizontal::default()).push(text::body(tr("Language")));
        let preference = i18n::preference();
        for (index, label) in language_options().into_iter().enumerate() {
            let label = if i18n::CODES[index] == preference { format!("✓ {label}") } else { label };
            actions = actions.push(menu_button(text::body(label)).on_press(Message::Language(index)));
        }
        if let Some(error) = &self.error {
            actions = actions.push(text::body(error.clone()));
        }
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
            ..Default::default()
        };
        control.refresh();
        let task = if standalone {
            control.set_header_title(tr("Maprisma Control"));
            control.set_window_title(tr("Maprisma Control"), control.core.main_window_id().unwrap())
        } else { Task::none() };
        (control, task)
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
            return cosmic::widget::scrollable(self.menu()).into();
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

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        self.core.applet.popup_container(self.menu()).into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
