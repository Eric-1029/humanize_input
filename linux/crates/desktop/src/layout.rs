//! The keyboard layout KDE is using right now.

use futures_util::StreamExt;
use humanize_keymap::XkbNames;

use crate::DesktopEvent;

#[zbus::proxy(
    interface = "org.kde.KeyboardLayouts",
    default_service = "org.kde.keyboard",
    default_path = "/Layouts"
)]
trait KeyboardLayouts {
    #[zbus(name = "getLayout")]
    fn get_layout(&self) -> zbus::Result<u32>;
    #[zbus(name = "getLayoutsList")]
    fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;
    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;
}

async fn names_for(proxy: &KeyboardLayoutsProxy<'_>, index: u32) -> Option<XkbNames> {
    let list = proxy.get_layouts_list().await.ok()?;
    let (layout, variant, _display) = list.get(usize::try_from(index).ok()?)?.clone();
    // Model and options are not on the bus; take them from kxkbrc.
    let base = XkbNames::from_kde(index as usize);
    Some(XkbNames { layout, variant, ..base })
}

/// The active layout over DBus, falling back to kxkbrc / system default.
pub(crate) async fn active(conn: &zbus::Connection) -> XkbNames {
    let from_bus = async {
        let proxy = KeyboardLayoutsProxy::new(conn).await.ok()?;
        let index = proxy.get_layout().await.ok()?;
        names_for(&proxy, index).await
    };
    match from_bus.await {
        Some(names) => names,
        None => XkbNames::from_kde(0),
    }
}

/// Reports layout switches until the connection closes.
pub(crate) async fn watch(conn: zbus::Connection, events: async_channel::Sender<DesktopEvent>) {
    let Ok(proxy) = KeyboardLayoutsProxy::new(&conn).await else { return };
    let Ok(mut changes) = proxy.receive_layout_changed().await else { return };
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if let Some(names) = names_for(&proxy, args.index).await
            && events.send(DesktopEvent::LayoutChanged(names)).await.is_err()
        {
            return;
        }
    }
}
