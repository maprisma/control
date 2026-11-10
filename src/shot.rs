//! F12 (without Ctrl, Shift, Alt or Super) while a popup or window of this app has the keyboard:
//! a picture of that surface, drawn again by iced itself (`window::screenshot`, no screenshot
//! program), saved by shot_files.rs, then a note card in the same surface: "What is wrong?
//! (optional)"; Enter or Done saves it next to the picture as a .txt, Esc skips it, closing the
//! popup keeps what was typed. One at a time: F12 waits until the card is closed.

use std::path::PathBuf;
use std::sync::LazyLock;

use cosmic::app::Task;
use cosmic::iced::futures::channel::oneshot;
use cosmic::iced::keyboard::{self, key::Named};
use cosmic::iced::widget::container as iced_container;
use cosmic::iced::window::{self, Screenshot};
use cosmic::iced::{Alignment, Background, Border, Length, Subscription, event};
use cosmic::widget::{self, button, column, container, row, space, text};
use cosmic::{Element, theme};

use crate::shot_files;
use crate::tr;

static NOTE_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("shot-note"));

#[derive(Debug, Clone)]
pub enum ShotMsg {
    Key(window::Id),
    Escape(window::Id),
    Captured(window::Id, Screenshot),
    Saved(window::Id, Result<PathBuf, String>),
    Text(String),
    Done,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Busy(window::Id),
    Note { window: window::Id, path: PathBuf, shown: String, text: String },
    Error { window: window::Id, text: String },
}

pub struct Shot {
    app_name: &'static str,
    phase: Phase,
}

impl Default for Shot {
    fn default() -> Self {
        Self::new("")
    }
}

fn why(error: &std::io::Error) -> String {
    let text = error.to_string();
    match text.find(" (os error ") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

impl Shot {
    /// `app_name` names the folder in ~/Pictures and (as a slug) the files.
    pub const fn new(app_name: &'static str) -> Self {
        Self { app_name, phase: Phase::Idle }
    }

    /// F12 and Esc, in whichever surface of the app has the keyboard.
    pub fn subscription() -> Subscription<ShotMsg> {
        event::listen_with(|event, _status, id| match event {
            cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                let plain = !(modifiers.control() || modifiers.shift() || modifiers.alt() || modifiers.logo());
                match key {
                    keyboard::Key::Named(Named::F12) if plain => Some(ShotMsg::Key(id)),
                    keyboard::Key::Named(Named::Escape) => Some(ShotMsg::Escape(id)),
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// The note card (or an error) is showing in this surface.
    pub fn showing(&self, id: window::Id) -> bool {
        matches!(&self.phase, Phase::Note { window, .. } | Phase::Error { window, .. } if *window == id)
    }

    /// `enabled`: Settings › F12 saves a screenshot; `ours`: the surface may be taken (the open
    /// popup, or the app's window), not the panel button.
    pub fn update<M: Clone + Send + 'static>(
        &mut self,
        message: ShotMsg,
        enabled: bool,
        ours: bool,
        wrap: fn(ShotMsg) -> M,
    ) -> Task<M> {
        match message {
            ShotMsg::Key(id) => {
                if enabled && ours && matches!(self.phase, Phase::Idle) {
                    self.phase = Phase::Busy(id);
                    return window::screenshot(id).map(move |shot| cosmic::Action::App(wrap(ShotMsg::Captured(id, shot))));
                }
            }
            ShotMsg::Captured(id, shot) => {
                if matches!(self.phase, Phase::Busy(busy) if busy == id) {
                    let name = self.app_name;
                    return cosmic::task::future(async move {
                        let (tx, rx) = oneshot::channel();
                        std::thread::spawn(move || {
                            let saved = shot_files::save_screenshot(name, &shot.rgba, shot.size.width, shot.size.height)
                                .map_err(|e| why(&e));
                            let _ = tx.send(saved);
                        });
                        let saved = rx.await.unwrap_or_else(|_| Err("the picture could not be written".into()));
                        wrap(ShotMsg::Saved(id, saved))
                    });
                }
            }
            ShotMsg::Saved(id, saved) => {
                if matches!(self.phase, Phase::Busy(busy) if busy == id) {
                    match saved {
                        Ok(path) => {
                            let shown = shot_files::shown(&path);
                            self.phase = Phase::Note { window: id, path, shown, text: String::new() };
                            return widget::text_input::focus(NOTE_ID.clone());
                        }
                        Err(error) => {
                            self.phase = Phase::Error {
                                window: id,
                                text: tr("The screenshot could not be saved: {0}").to_string().replace("{0}", &error),
                            };
                        }
                    }
                }
            }
            ShotMsg::Text(value) => {
                if let Phase::Note { text, .. } = &mut self.phase {
                    *text = value.chars().take(shot_files::NOTE_LIMIT).collect();
                }
            }
            ShotMsg::Done => self.finish(true),
            ShotMsg::Escape(id) => {
                if self.showing(id) {
                    self.finish(false);
                }
            }
        }
        Task::none()
    }

    /// The surface closes: a typed note is still saved; a capture in progress is dropped.
    pub fn closed(&mut self, id: window::Id) {
        match &self.phase {
            Phase::Busy(busy) if *busy == id => self.phase = Phase::Idle,
            Phase::Note { window, .. } | Phase::Error { window, .. } if *window == id => self.finish(true),
            _ => {}
        }
    }

    fn finish(&mut self, keep: bool) {
        match std::mem::take(&mut self.phase) {
            Phase::Note { window, path, text, .. } => {
                if keep && let Err(error) = shot_files::save_note(&path, &text) {
                    self.phase = Phase::Error {
                        window,
                        text: tr("The note could not be saved: {0}").to_string().replace("{0}", &why(&error)),
                    };
                }
            }
            Phase::Busy(id) => self.phase = Phase::Busy(id),
            Phase::Idle | Phase::Error { .. } => {}
        }
    }

    /// The card for this surface, shown above its content while a note is asked for.
    pub fn card<'a, M: Clone + 'static>(&'a self, id: window::Id, wrap: fn(ShotMsg) -> M) -> Option<Element<'a, M>> {
        let mut body = column![].spacing(8);
        match &self.phase {
            Phase::Note { window, shown, text: note, .. } if *window == id => {
                body = body
                    .push(text::heading(tr("Screenshot saved").to_string()))
                    .push(text::caption(tr("Saved as {0}").to_string().replace("{0}", shown)))
                    .push(
                        widget::text_input(tr("What is wrong? (optional)").to_string(), note.as_str())
                            .id(NOTE_ID.clone())
                            .on_input(move |value| wrap(ShotMsg::Text(value)))
                            .on_submit(move |_| wrap(ShotMsg::Done)),
                    );
            }
            Phase::Error { window, text: error } if *window == id => {
                body = body.push(text::body(error.as_str()).class(theme::Text::Accent));
            }
            _ => return None,
        }
        body = body.push(
            row![space::horizontal().width(Length::Fill), button::suggested(tr("Done").to_string()).on_press(wrap(ShotMsg::Done))]
                .align_y(Alignment::Center),
        );
        let card = container(body).padding(12).width(Length::Fill).class(theme::Container::custom(|t: &cosmic::Theme| {
            let c = t.cosmic().primary(t.transparent);
            let radius = t.cosmic().corner_radii.radius_m;
            iced_container::Style {
                text_color: Some(c.on.into()),
                background: Some(Background::Color(c.base.into())),
                border: Border { radius: radius.into(), width: 1.0, color: c.divider.into() },
                ..Default::default()
            }
        }));
        Some(container(card).padding([8, 8, 0, 8]).into())
    }
}
