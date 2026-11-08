mod model;
use maprisma_applet::*;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use zbus::{
    Connection,
    object_server::SignalEmitter,
    zvariant::{OwnedObjectPath, OwnedValue, Value as Variant},
};

type Props = HashMap<String, OwnedValue>;
type Layout = (i32, Props, Vec<OwnedValue>);
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
const ICON: &str = "com.maprisma.Maprisma";
struct State {
    running: Option<Running>,
    pending: Option<(Pending, std::time::Instant)>,
    menu: model::Item,
    revision: u32,
}
impl State {
    fn refresh(&mut self) {
        self.running = scan();
        settle_pending(&self.running, &mut self.pending);
        self.menu = model::menu(
            &self.running,
            self.pending.map(|p| p.0),
            &version(),
            last_run().is_some(),
            test_dir().join("start.sh").is_file(),
        );
    }
    fn status(&self) -> &'static str {
        model::status(&self.running)
    }
    fn tooltip(&self) -> String {
        status_text(&self.running, self.pending.map(|p| p.0))
    }
}
type Shared = Arc<Mutex<State>>;
#[derive(Clone)]
struct Menu {
    state: Shared,
    tx: mpsc::UnboundedSender<Action>,
}
fn owned<T: Into<Variant<'static>>>(value: T) -> OwnedValue {
    OwnedValue::try_from(value.into()).unwrap()
}
fn properties(item: &model::Item, names: &[String]) -> Props {
    let mut props = Props::new();
    props.insert("label".into(), owned(item.label.clone()));
    props.insert("enabled".into(), owned(item.enabled));
    props.insert("visible".into(), owned(true));
    if !item.children.is_empty() {
        props.insert("children-display".into(), owned("submenu"));
    }
    if item.separator {
        props.insert("type".into(), owned("separator"));
    }
    if !names.is_empty() {
        props.retain(|key, _| names.contains(key));
    }
    props
}
fn layout(item: &model::Item, depth: i32, names: &[String]) -> Layout {
    (
        item.id,
        properties(item, names),
        if depth == 0 {
            vec![]
        } else {
            item.children
                .iter()
                .map(|c| {
                    OwnedValue::try_from(Variant::new(layout(
                        c,
                        if depth < 0 { -1 } else { depth - 1 },
                        names,
                    )))
                    .unwrap()
                })
                .collect()
        },
    )
}
impl Menu {
    async fn dispatch(&self, id: i32, event: &str) -> bool {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        let Some(item) = menu.find(id) else {
            return false;
        };
        if event == "clicked" {
            if let Some(action) = menu.activate(item.id) {
                let _ = self.tx.send(action);
            }
        }
        true
    }
}
#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }
    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        let item = menu
            .find(parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Unknown menu ID".into()))?;
        Ok((s.revision, layout(item, recursion_depth, &property_names)))
    }
    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        ids.iter()
            .filter_map(|id| {
                menu.find(*id)
                    .map(|i| (*id, properties(i, &property_names)))
            })
            .collect()
    }
    async fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        menu.find(id)
            .and_then(|i| properties(i, &[]).remove(&name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Unknown menu property".into()))
    }
    async fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        self.dispatch(id, &event_id).await;
    }
    async fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let mut errors = vec![];
        for (id, event, _, _) in events {
            if !self.dispatch(id, &event).await {
                errors.push(id);
            }
        }
        errors
    }
    async fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    async fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        (
            vec![],
            ids.into_iter()
                .filter(|id| menu.find(*id).is_none())
                .collect(),
        )
    }
    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}
struct Tray {
    state: Shared,
    tx: mpsc::UnboundedSender<Action>,
}
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Tray {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "maprisma-control"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Maprisma Control"
    }
    #[zbus(property)]
    async fn status(&self) -> &str {
        self.state.lock().await.status()
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ICON
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        vec![]
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Pixmaps {
        vec![]
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ICON
    }
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Pixmaps {
        vec![]
    }
    #[zbus(property)]
    fn attention_movie_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Pixmaps, String, String) {
        (
            ICON.into(),
            vec![],
            "Maprisma Control".into(),
            self.state.lock().await.tooltip(),
        )
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/MenuBar").unwrap()
    }
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Action::Activate);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Action::StartOrStop);
    }
    fn context_menu(&self, _x: i32, _y: i32) {} // The host displays the exported Menu.
    fn scroll(&self, _delta: i32, _orientation: String) {}
    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
async fn publish(
    connection: &Connection,
    state: &Shared,
    action: Option<Action>,
) -> zbus::Result<()> {
    let mut s = state.lock().await;
    let before = s.menu.clone();
    let old_status = s.status();
    let old_tooltip = s.tooltip();
    s.refresh();
    if let Some(action) = action {
        let running = s.running.clone();
        execute(action, &running, &mut s.pending);
        s.menu = model::menu(
            &s.running,
            s.pending.map(|p| p.0),
            &version(),
            last_run().is_some(),
            test_dir().join("start.sh").is_file(),
        );
    }
    let changed = before != s.menu;
    if changed {
        s.revision = s.revision.wrapping_add(1);
    }
    let revision = s.revision;
    let status = s.status();
    let tooltip_changed = old_tooltip != s.tooltip();
    drop(s);
    if changed {
        Menu::layout_updated(&SignalEmitter::new(connection, "/MenuBar")?, revision, 0).await?;
    }
    if tooltip_changed {
        Tray::new_tool_tip(&SignalEmitter::new(connection, "/StatusNotifierItem")?).await?;
    }
    if status != old_status {
        Tray::new_status(
            &SignalEmitter::new(connection, "/StatusNotifierItem")?,
            status,
        )
        .await?;
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut initial = State {
        running: None,
        pending: None,
        menu: model::menu(&None, None, "", false, false),
        revision: 1,
    };
    initial.refresh();
    let state = Arc::new(Mutex::new(initial));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let connection = zbus::connection::Builder::session()?
        .serve_at(
            "/StatusNotifierItem",
            Tray {
                state: state.clone(),
                tx: tx.clone(),
            },
        )?
        .serve_at(
            "/MenuBar",
            Menu {
                state: state.clone(),
                tx,
            },
        )?
        .build()
        .await?;
    // One tray per session: a second one ends here (zbus would otherwise
    // queue the name request and keep running).
    if connection.request_name_with_flags("com.maprisma.Control.Tray", zbus::fdo::RequestNameFlags::DoNotQueue.into()).await.is_err() {
        return Ok(());
    }
    // Checking the unique owner also handles Plasma starting after us and watcher restarts.
    let watcher_connection = connection.clone();
    tokio::spawn(async move {
        let mut registered = String::new();
        loop {
            let dbus = zbus::fdo::DBusProxy::new(&watcher_connection).await;
            let owner = match dbus {
                Ok(p) => p
                    .get_name_owner("org.kde.StatusNotifierWatcher".try_into().unwrap())
                    .await
                    .ok()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            if owner.is_empty() {
                registered.clear();
            } else if owner != registered {
                if let Ok(proxy) = zbus::Proxy::new(
                    &watcher_connection,
                    owner.as_str(),
                    "/StatusNotifierWatcher",
                    "org.kde.StatusNotifierWatcher",
                )
                .await
                {
                    if proxy
                        .call::<_, _, ()>(
                            "RegisterStatusNotifierItem",
                            &(watcher_connection.unique_name().unwrap().as_str(),),
                        )
                        .await
                        .is_ok()
                    {
                        registered = owner;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    let mut poll = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _=poll.tick()=> {
                publish(&connection, &state, None).await?;
            }
            Some(action)=rx.recv()=> { publish(&connection, &state, Some(action)).await?; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dbus_layout_signature_depth_and_properties() {
        let menu = model::menu(&None, None, "", false, true);
        assert_eq!(
            Variant::new(layout(&menu, -1, &[]))
                .value_signature()
                .to_string(),
            "(ia{sv}av)"
        );
        assert!(layout(&menu, 0, &[]).2.is_empty());
        assert_eq!(layout(&menu, 1, &[]).2.len(), menu.children.len());
        let props = properties(menu.find(5).unwrap(), &["type".into()]);
        assert_eq!(props.len(), 1);
        assert_eq!(<&str>::try_from(&props["type"]).unwrap(), "separator");
    }
    #[tokio::test]
    async fn events_and_groups_without_bus() {
        let state = Arc::new(Mutex::new(State {
            running: None,
            pending: None,
            menu: model::menu(&None, None, "", false, true),
            revision: 1,
        }));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let menu = Menu {
            state: state.clone(),
            tx: tx.clone(),
        };
        menu.event(8, "clicked".into(), owned(0i32), 0).await;
        assert_eq!(rx.try_recv().unwrap(), Action::Start);
        assert_eq!(
            menu.event_group(vec![
                (9, "clicked".into(), owned(0i32), 0),
                (-1, "clicked".into(), owned(0i32), 0),
                (11, "clicked".into(), owned(0i32), 0)
            ])
            .await,
            vec![-1]
        );
        assert_eq!(rx.try_recv().unwrap(), Action::StartRecorded);
        assert!(rx.try_recv().is_err());
        menu.event(8, "hovered".into(), owned(0i32), 0).await;
        assert!(rx.try_recv().is_err());
        let tray = Tray { state, tx };
        tray.activate(0, 0);
        tray.secondary_activate(0, 0);
        assert_eq!(rx.try_recv().unwrap(), Action::Activate);
        assert_eq!(rx.try_recv().unwrap(), Action::StartOrStop);
    }
}
