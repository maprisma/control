use maprisma_applet::{Action, Pending, Running, status_text, tr, trf, language_options};
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub children: Vec<Item>,
    pub action: Option<Action>,
}
impl Item {
    fn new(id: i32, label: impl Into<String>, enabled: bool, action: Option<Action>) -> Self {
        Self {
            id,
            label: label.into(),
            enabled,
            action,
            separator: false,
            children: vec![],
        }
    }
    pub fn find(&self, id: i32) -> Option<&Self> {
        if self.id == id {
            Some(self)
        } else {
            self.children.iter().find_map(|c| c.find(id))
        }
    }
    pub fn activate(&self, id: i32) -> Option<Action> {
        self.find(id).filter(|i| i.enabled).and_then(|i| i.action)
    }
}
pub fn status(running: &Option<Running>) -> &'static str {
    if running.as_ref().is_some_and(|r| r.recording.is_some()) {
        "NeedsAttention"
    } else {
        "Active"
    }
}
pub fn menu(
    running: &Option<Running>,
    pending: Option<Pending>,
    version: &str,
    has_run: bool,
    has_script: bool,
) -> Item {
    let mut items = vec![
        Item::new(
            1,
            if version.is_empty() {
                "Maprisma".into()
            } else {
                format!("Maprisma {version}")
            },
            false,
            None,
        ),
        Item::new(2, status_text(running, pending), false, None),
    ];
    if let Some(dir) = running.as_ref().and_then(|r| r.recording.as_ref()) {
        items.push(Item::new(
            3,
            trf("Recording into {0}", &[&dir.display().to_string()]),
            false,
            None,
        ));
        items.push(Item::new(
            4,
            tr("F12 in Maprisma: a screenshot with a note. Stop or close it to finish."),
            false,
            None,
        ));
    }
    let mut sep = Item::new(5, "", false, None);
    sep.separator = true;
    items.push(sep);
    let idle = pending.is_none();
    if running.is_some() {
        items.push(Item::new(
            6,
            tr("Show / hide window"),
            idle,
            Some(Action::ToggleWindow),
        ));
        items.push(Item::new(7, tr("Stop Maprisma"), idle, Some(Action::Stop)));
    } else {
        items.push(Item::new(8, tr("Start Maprisma"), idle, Some(Action::Start)));
        items.push(Item::new(
            9,
            tr("Start a recorded test session"),
            idle && has_script,
            Some(Action::StartRecorded),
        ));
    }
    let mut sep = Item::new(10, "", false, None);
    sep.separator = true;
    items.push(sep);
    items.push(Item::new(
        11,
        tr("Open notes of the last test"),
        has_run,
        Some(Action::OpenNotes),
    ));
    items.push(Item::new(
        12,
        tr("Open the test folder"),
        true,
        Some(Action::OpenFolder),
    ));
    items.push(Item::new(13, tr("Open Maprisma Control"), true, Some(Action::OpenControl)));
    let mut languages = Item::new(20, tr("Language"), true, None);
    let preference = maprisma_applet::i18n::preference();
    languages.children = language_options().into_iter().enumerate().map(|(index, label)| {
        let label = if maprisma_applet::i18n::CODES[index] == preference { format!("✓ {label}") } else { label };
        Item::new(100 + index as i32, label, true, Some(Action::Language(index)))
    }).collect();
    items.push(languages);
    let mut root = Item::new(0, "", true, None);
    root.children = items;
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_matches_applet_states() {
        let m = menu(&None, None, "1.2", false, false);
        assert_eq!(m.find(1).unwrap().label, "Maprisma 1.2");
        assert_eq!(m.activate(8), Some(Action::Start));
        assert!(m.activate(9).is_none());
        assert!(m.activate(11).is_none());
        assert!(m.find(6).is_none());
        // Corners and F12 live in Control's window: the tray opens it.
        assert_eq!(m.activate(13), Some(Action::OpenControl));
        let m = menu(&None, None, "", true, true);
        assert_eq!(m.activate(9), Some(Action::StartRecorded));
        assert_eq!(m.activate(11), Some(Action::OpenNotes));
        let r = Some(Running {
            recording: Some("/tmp/run".into()),
            ..Default::default()
        });
        assert_eq!(status(&r), "NeedsAttention");
        assert_eq!(status(&None), "Active");
        let m = menu(&r, None, "", true, true);
        assert_eq!(m.activate(6), Some(Action::ToggleWindow));
        assert_eq!(m.activate(7), Some(Action::Stop));
        assert!(m.find(9).is_none());
        assert!(m.find(3).unwrap().label.contains("/tmp/run"));
        for pending in [Pending::Starting, Pending::Stopping] {
            let m = menu(&r, Some(pending), "", true, true);
            assert!(m.activate(6).is_none());
            assert!(m.activate(7).is_none());
            assert_eq!(m.activate(12), Some(Action::OpenFolder));
            assert_eq!(m.activate(11), Some(Action::OpenNotes));
            assert!(m.activate(1).is_none());
        }
    }
}
