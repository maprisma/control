//! Host-independent SNI icons. IconName is empty so hosts use these ARGB pixmaps.
use super::{Pixmaps, look};
use resvg::{tiny_skia, usvg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub stopped: [u8; 3],
    pub active: [u8; 3],
}

pub fn choose(dark: Option<bool>, active: Option<[u8; 3]>) -> Colors {
    Colors {
        stopped: if dark == Some(false) {
            [35, 38, 41]
        } else {
            [239, 241, 245]
        },
        active: active.unwrap_or([71, 190, 112]),
    }
}

pub async fn colors(connection: &zbus::Connection) -> Colors {
    let desktop = look::desktop();
    if desktop == look::Desktop::Cosmic {
        let theme = cosmic::theme::system_preference();
        let active: cosmic::iced::Color = theme.cosmic().success_color().into();
        return choose(
            Some(theme.cosmic().is_dark),
            Some([
                (active.r * 255.0).round() as u8,
                (active.g * 255.0).round() as u8,
                (active.b * 255.0).round() as u8,
            ]),
        );
    }
    let palette = look::read_palette(desktop);
    let portal = async {
        let proxy = zbus::Proxy::new(
            connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .await
        .ok()?;
        let value: zbus::zvariant::OwnedValue = proxy
            .call("Read", &("org.freedesktop.appearance", "color-scheme"))
            .await
            .ok()?;
        match u32::try_from(value).ok()? {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        }
    };
    let dark = palette
        .as_ref()
        .and_then(|p| p.background.map(|_| p.is_dark()))
        .or(
            tokio::time::timeout(std::time::Duration::from_millis(300), portal)
                .await
                .ok()
                .flatten(),
        );
    let active = palette.as_ref().and_then(|p| p.green.or(p.accent));
    choose(dark, active)
}

pub fn pixmaps(colors: Colors, running: bool) -> Pixmaps {
    let rgb = if running {
        colors.active
    } else {
        colors.stopped
    };
    let source = include_str!("../../data/icons/maprisma-off-symbolic.svg").replace(
        "#232629",
        &format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]),
    );
    let tree =
        usvg::Tree::from_str(&source, &usvg::Options::default()).expect("embedded robot SVG");
    [16, 22, 24, 32, 48]
        .into_iter()
        .map(|size| {
            let mut image = tiny_skia::Pixmap::new(size, size).unwrap();
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(
                    size as f32 / tree.size().width(),
                    size as f32 / tree.size().height(),
                ),
                &mut image.as_mut(),
            );
            // A one-pixel contrasting halo protects against panels whose scheme differs
            // from the desktop preference (and against an unknown scheme).
            let brightness = 0.2126 * f32::from(rgb[0])
                + 0.7152 * f32::from(rgb[1])
                + 0.0722 * f32::from(rgb[2]);
            let outline: [u8; 3] = if brightness > 128.0 {
                [24, 28, 34]
            } else {
                [245, 245, 245]
            };
            let mut argb = Vec::with_capacity((size * size * 4) as usize);
            for y in 0..size {
                for x in 0..size {
                    let pixel = image.pixel(x, y).unwrap().demultiply();
                    let mut halo = 0;
                    for dy in -1i32..=1 {
                        for dx in -1i32..=1 {
                            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                            if nx >= 0 && ny >= 0 && nx < size as i32 && ny < size as i32 {
                                halo = halo.max(image.pixel(nx as u32, ny as u32).unwrap().alpha());
                            }
                        }
                    }
                    let a = f32::from(pixel.alpha()) / 255.0;
                    let h = f32::from(halo) / 255.0 * 0.8 * (1.0 - a);
                    let alpha = a + h;
                    argb.push((alpha * 255.0).round() as u8);
                    for (fg, bg) in [pixel.red(), pixel.green(), pixel.blue()]
                        .into_iter()
                        .zip(outline)
                    {
                        argb.push(if alpha > 0.0 {
                            ((f32::from(fg) * a + f32::from(bg) * h) / alpha).round() as u8
                        } else {
                            0
                        });
                    }
                }
            }
            (size as i32, size as i32, argb)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_choice() {
        assert_eq!(choose(None, None), choose(Some(true), None));
        assert_ne!(
            choose(Some(false), None).stopped,
            choose(Some(true), None).stopped
        );
        assert_eq!(choose(Some(true), Some([1, 2, 3])).active, [1, 2, 3]);
        assert_ne!(choose(None, None).active, choose(None, None).stopped);
    }
    #[test]
    fn render_previews_and_check_argb() {
        let dir = std::path::Path::new("target/tray-contrast");
        std::fs::create_dir_all(dir).unwrap();
        for (scheme, dark) in [
            ("dark", Some(true)),
            ("light", Some(false)),
            ("unknown", None),
        ] {
            for running in [false, true] {
                for (w, h, bytes) in pixmaps(choose(dark, None), running) {
                    assert_eq!(bytes.len(), (w * h * 4) as usize);
                    let pixels = bytes.as_chunks::<4>().0;
                    assert!(pixels.iter().any(|p| p[0] == 255));
                    let expected = if running {
                        choose(dark, None).active
                    } else {
                        choose(dark, None).stopped
                    };
                    assert!(pixels.iter().any(|p| p[0] == 255 && p[1..] == expected));
                    for (background, bg) in [("dark", [30u8, 34, 44]), ("light", [245u8, 245, 245])]
                    {
                        let mut preview = tiny_skia::Pixmap::new(w as u32, h as u32).unwrap();
                        for (out, p) in preview
                            .data_mut()
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip(pixels)
                        {
                            for c in 0..3 {
                                out[c] = ((u32::from(p[c + 1]) * u32::from(p[0])
                                    + u32::from(bg[c]) * (255 - u32::from(p[0])))
                                    / 255) as u8;
                            }
                            out[3] = 255;
                        }
                        preview
                            .save_png(
                                dir.join(format!("{scheme}-{running}-{w}-on-{background}.png")),
                            )
                            .unwrap();
                    }
                }
            }
        }
    }
}
