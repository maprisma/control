//! Settings › Corners: "Like the desktop (N px)" (COSMIC's window corners), "Slightly rounded
//! (6 px)", "Rounded (12 px)" or "Square", in Maprisma's three sizes plus pills: containers
//! (popups, cards, drop-down menus) take the radius, buttons, drop-downs and fields at most 8,
//! small parts (check boxes, slider tracks, bars) at most 4, pills 999 or 0 in Square.
//!
//! libcosmic's widgets take their corners from the theme, so the app runs on a copy of the
//! desktop's theme with these corner radii (slots: radius_xs small, radius_s and radius_xl
//! controls, radius_m and radius_l containers). The copy is made again when the desktop's theme
//! changes.
#![allow(dead_code)] // shared by several apps; each uses a part

use std::sync::Arc;

use cosmic::app::{Core, Task};
use cosmic::cosmic_theme::{CornerRadii, Theme as CosmicTheme};
use cosmic::theme::ThemeType;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Corners {
    #[default]
    Desktop,
    Slight,
    Rounded,
    Square,
}

pub const ALL: [Corners; 4] = [Corners::Desktop, Corners::Slight, Corners::Rounded, Corners::Square];

impl Corners {
    /// The stored value; anything unknown (or missing) is Like the desktop.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "slight" => Self::Slight,
            "rounded" => Self::Rounded,
            "square" => Self::Square,
            _ => Self::Desktop,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Slight => "slight",
            Self::Rounded => "rounded",
            Self::Square => "square",
        }
    }

    pub fn index(self) -> usize {
        ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// The radius in px, `desktop` being the desktop's window corners.
    pub fn radius(self, desktop: f32) -> f32 {
        match self {
            Self::Desktop => desktop.max(0.0),
            Self::Slight => 6.0,
            Self::Rounded => 12.0,
            Self::Square => 0.0,
        }
    }
}

/// The sizes of one radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    pub radius: f32,
    pub control: f32,
    pub small: f32,
    pub pill: f32,
}

pub fn sizes(radius: f32) -> Sizes {
    let radius = radius.max(0.0);
    Sizes { radius, control: radius.min(8.0), small: radius.min(4.0), pill: if radius > 0.0 { 999.0 } else { 0.0 } }
}

/// The sizes in effect: read back from the theme the app runs on.
pub fn current(theme: &cosmic::Theme) -> Sizes {
    sizes(theme.cosmic().corner_radii.radius_m[0])
}

/// COSMIC's window corners, as libcosmic rounds windows: radius_s, plus 4 when it is 4 or more.
pub fn desktop_radius(theme: &CosmicTheme) -> f32 {
    let s = theme.corner_radii.radius_s[0];
    if s < 4.0 { s } else { s + 4.0 }
}

pub fn radii(radius: f32) -> CornerRadii {
    let s = sizes(radius);
    CornerRadii {
        radius_0: [0.0; 4],
        radius_xs: [s.small; 4],
        radius_s: [s.control; 4],
        radius_m: [s.radius; 4],
        radius_l: [s.radius; 4],
        radius_xl: [s.control; 4],
    }
}

/// The desktop's theme with the corners of `corners`.
pub fn themed(base: &CosmicTheme, corners: Corners) -> CosmicTheme {
    let mut theme = base.clone();
    theme.corner_radii = radii(corners.radius(desktop_radius(base)));
    theme
}

/// Keeps the app's theme on the desktop's theme with the chosen corners.
#[derive(Default)]
pub struct Look {
    base: Option<Arc<CosmicTheme>>,
    forced_dark: Option<bool>,
    made: Option<Arc<CosmicTheme>>,
    last_system: Option<CosmicTheme>,
}

impl Look {
    /// The desktop's window corners (for "Like the desktop (N px)").
    pub fn desktop(&self) -> f32 {
        desktop_radius(self.base.as_deref().unwrap_or(cosmic::theme::active().cosmic()))
    }

    /// The task that puts `corners` into the app's theme; none when it is there already.
    pub fn apply<M: Send + 'static>(&mut self, core: &Core, corners: Corners) -> Task<M> {
        let active = cosmic::theme::active();
        match &active.theme_type {
            // Ours: the base stays; the desktop's theme may have changed meanwhile.
            ThemeType::Custom(theme) if self.made.as_ref().is_some_and(|m| Arc::ptr_eq(m, theme)) => {
                let system = core.system_theme().cosmic();
                if self.last_system.as_ref() != Some(system) {
                    self.last_system = Some(system.clone());
                    if self.forced_dark.is_none_or(|dark| dark == system.is_dark) {
                        self.base = Some(Arc::new(system.clone()));
                    }
                }
            }
            // libcosmic's (the start, or the desktop's theme changed): the new base.
            other => {
                if let ThemeType::System { prefer_dark, .. } = other {
                    self.forced_dark = *prefer_dark;
                }
                self.base = Some(Arc::new(active.cosmic().clone()));
                self.last_system = Some(core.system_theme().cosmic().clone());
            }
        }
        let Some(base) = self.base.as_deref() else { return Task::none() };
        let wanted = themed(base, corners);
        if active.cosmic() == &wanted {
            return Task::none();
        }
        let made = Arc::new(wanted);
        self.made = Some(made.clone());
        cosmic::command::set_theme(cosmic::Theme::custom(made))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

fn same_except_corners(a: &CosmicTheme, b: &CosmicTheme) -> bool {
    let mut b = b.clone();
    b.corner_radii = a.corner_radii;
    *a == b
}


    #[test]
    fn corner_sizes() {
        assert_eq!(sizes(12.0), Sizes { radius: 12.0, control: 8.0, small: 4.0, pill: 999.0 });
        assert_eq!(sizes(6.0), Sizes { radius: 6.0, control: 6.0, small: 4.0, pill: 999.0 });
        assert_eq!(sizes(2.0), Sizes { radius: 2.0, control: 2.0, small: 2.0, pill: 999.0 });
        assert_eq!(sizes(0.0), Sizes { radius: 0.0, control: 0.0, small: 0.0, pill: 0.0 });
        assert_eq!(Corners::Slight.radius(16.0), 6.0);
        assert_eq!(Corners::Rounded.radius(2.0), 12.0);
        assert_eq!(Corners::Square.radius(12.0), 0.0);
        assert_eq!(Corners::Desktop.radius(12.0), 12.0);
        let r = radii(12.0);
        assert_eq!((r.radius_xs[0], r.radius_s[0], r.radius_m[0], r.radius_l[0], r.radius_xl[0]), (4.0, 8.0, 12.0, 12.0, 8.0));
        assert_eq!(radii(0.0).radius_xl, [0.0; 4]);
    }

    #[test]
    fn stored_values() {
        for corners in ALL {
            assert_eq!(Corners::parse(corners.key()), corners);
            assert_eq!(ALL[corners.index()], corners);
        }
        assert_eq!(Corners::parse(""), Corners::Desktop);
        assert_eq!(Corners::parse("windows"), Corners::Desktop);
        assert_eq!(Corners::parse("bogus"), Corners::Desktop);
    }

    #[test]
    fn desktop_corners_follow_cosmic() {
        // COSMIC's styles: Round (radius_s 8), Slightly round (8), Square (2).
        let mut theme = CosmicTheme::default();
        theme.corner_radii.radius_s = [8.0; 4];
        assert_eq!(desktop_radius(&theme), 12.0);
        theme.corner_radii.radius_s = [2.0; 4];
        assert_eq!(desktop_radius(&theme), 2.0);
        let made = themed(&theme, Corners::Desktop);
        assert_eq!(made.corner_radii.radius_m, [2.0; 4]);
        assert_eq!(made.corner_radii.radius_xs, [2.0; 4]);
        // Only the corners change.
        assert!(same_except_corners(&theme, &themed(&theme, Corners::Rounded)));
    }
}
