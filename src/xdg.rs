//! This module contains XDG and DBus specific code.
//!
//! it should not be available under any platform other than `(unix, not(target_os = "macos"))`
use dbus::{
    arg::messageitem::MessageItem,
    Message,
    ffidisp::{BusType, Connection, ConnectionItem}
};


use crate::{
    error::*,
    notification::Notification
};

use std::ops::{Deref, DerefMut};

#[cfg(not(feature = "debug_namespace"))] pub static NOTIFICATION_NAMESPACE: &str = "org.freedesktop.Notifications";
#[cfg(not(feature = "debug_namespace"))] pub static NOTIFICATION_OBJECTPATH: &str = "/org/freedesktop/Notifications";

#[cfg(feature = "debug_namespace")] pub static NOTIFICATION_NAMESPACE: &str = "de.hoodie.Notifications";
#[cfg(feature = "debug_namespace")] pub static NOTIFICATION_OBJECTPATH: &str = "/de/hoodie/Notifications";

/// A handle to a shown notification.
///
/// This keeps a connection alive to ensure actions work on certain desktops.
#[derive(Debug)]
pub struct NotificationHandle {
    id:           u32,
    connection:   Connection,
    notification: Notification
}

impl NotificationHandle {
    pub(crate) fn new(id: u32, connection: Connection, notification: Notification) -> NotificationHandle {
        NotificationHandle { id, connection, notification }
    }

    /// Waits for the user to act on a notification and then calls
    /// `invocation_closure` with the name of the corresponding action.
    pub fn wait_for_action<F>(self, invocation_closure: F)
        where F: FnOnce(&str)
    {
        wait_for_action_signal(&self.connection, self.id, invocation_closure);
    }

    /// Manually close the notification
    /// 
    /// # Example
    /// see 
    /// ```no_run
    /// let handle: notify_rust::NotificationHandle = Notification::new()
    ///     .summary("oh no")
    ///     .hint(notify_rust::Hint::Transient(true))
    ///     .body("I'll be here till you close me!")
    ///     .hint(Hint::Resident(true)) // does not work on kde
    ///     .timeout(Timeout::Never) // works on kde and gnome
    ///     .show()
    ///     .unwrap();
    /// // ... and then later
    /// handle.close();
    /// ```
    pub fn close(self) {
        let mut message = build_message("CloseNotification");
        message.append_items(&[self.id.into()]);
        let _ = self.connection.send(message); // If closing fails there's nothing we could do anyway
    }

    /// Executes a closure after the notification has closed.
    /// ## Example
    /// ```no_run
    /// # use notify_rust::Notification;
    /// Notification::new().summary("Time is running out")
    ///                    .body("This will go away.")
    ///                    .icon("clock")
    ///                    .show()
    ///                    .unwrap()
    ///                    .on_close(|| println!("closed"));
    /// ```
    pub fn on_close<F>(self, closure: F)
        where F: FnOnce()
    {
        self.wait_for_action(|action| {
                                 if action == "__closed" {
                                     closure();
                                 }
                             });
    }

    /// Replace the original notification with an updated version
    /// ## Example
    /// ```no_run
    /// # use notify_rust::Notification;
    /// let mut notification = Notification::new().summary("Latest News")
    ///                                           .body("Bayern Dortmund 3:2")
    ///                                           .show()
    ///                                           .unwrap();
    ///
    /// std::thread::sleep_ms(1_500);
    ///
    /// notification.summary("Latest News (Correction)")
    ///             .body("Bayern Dortmund 3:3");
    ///
    /// notification.update();
    /// ```
    /// Watch out for different implementations of the
    /// notification server! On plasma5 for instance, you should also change the appname, so the old
    /// message is really replaced and not just amended. Xfce behaves well, all others have not
    /// been tested by the developer.
    pub fn update(&mut self) {
        self.id = show_notification_dbus(&self.notification, self.id, &self.connection).unwrap();
    }

    /// Returns the Handle's id.
    pub fn id(&self) -> u32 {
        self.id
    }
}

/// Required for `DerefMut`
impl Deref for NotificationHandle {
    type Target = Notification;

    fn deref(&self) -> &Notification {
        &self.notification
    }
}

/// Allow you to easily modify notification properties
impl DerefMut for NotificationHandle {
    fn deref_mut(&mut self) -> &mut Notification {
        &mut self.notification
    }
}

pub(crate) fn show_notification(notification: &Notification) -> Result<NotificationHandle> {
    let connection = Connection::get_private(BusType::Session)?;
    let inner_id = notification.id.unwrap_or(0);
    let id = show_notification_dbus(notification, inner_id, &connection)?;
    Ok(NotificationHandle::new(id, connection, notification.clone()))
}

fn show_notification_dbus(notification: &Notification, id: u32, connection: &Connection) -> Result<u32> {
    let mut message = build_message("Notify");
    let timeout: i32 = notification.timeout.into();
    message.append_items(&[notification.appname.to_owned().into(),  // appname
                            id.into(),                              // notification to update
                            notification.icon.to_owned().into(),    // icon
                            notification.summary.to_owned().into(), // summary (title)
                            notification.body.to_owned().into(),    // body
                            dbus_rs::pack_actions(notification),    // actions
                            dbus_rs::pack_hints(notification)?,     // hints
                            timeout.into()                          // timeout
    ]);

    let reply = connection.send_with_reply_and_block(message, 2000)?;

    match reply.get_items().get(0) {
        Some(&MessageItem::UInt32(ref id)) => Ok(*id),
        _ => Ok(0)
    }
}

mod dbus_rs {
    #![allow(unused_imports)]
    use super::*;
    use crate::hints::{Hint, message::HintMessage};
    use dbus::{arg::messageitem::{MessageItem, MessageItemArray}, ffidisp::{Connection, BusType} };

    pub fn pack_hints(notification: &Notification) -> Result<MessageItem> {
        if !notification.hints.is_empty() {
            let hints = notification.hints
                .iter()
                .cloned()
                .map(HintMessage::wrap_hint)
                .collect::<Vec<(MessageItem, MessageItem)>>();

            if let Ok(array) = MessageItem::new_dict(hints) {
                return Ok(array);
            }
        }

        Ok(MessageItem::Array(MessageItemArray::new(vec![], "a{sv}".into()).unwrap()))
    }

    pub fn pack_actions(notification: &Notification) -> MessageItem {
        if !notification.actions.is_empty() {
            let mut actions = vec![];
            for action in &notification.actions {
                actions.push(action.to_owned().into());
            }
            if let Ok(array) = MessageItem::new_array(actions) {
                return array;
            }
        }

        MessageItem::Array(MessageItemArray::new(vec![], "as".into()).unwrap())
    }
}

// here be public functions

/// Get list of all capabilities of the running notification server.
pub fn get_capabilities() -> Result<Vec<String>> {
    let mut capabilities = vec![];

    let message = build_message("GetCapabilities");
    let connection = Connection::get_private(BusType::Session)?;
    let reply = connection.send_with_reply_and_block(message, 2000)?;

    if let Some(&MessageItem::Array(ref items)) = reply.get_items().get(0) {
        for item in items.iter() {
            if let MessageItem::Str(ref cap) = *item {
                capabilities.push(cap.clone());
            }
        }
    }

    Ok(capabilities)
}

/// Returns a struct containing `ServerInformation`.
///
/// This struct contains `name`, `vendor`, `version` and `spec_version` of the notification server
/// running.
/// TODO dbus stuff module!!!
pub fn get_server_information() -> Result<ServerInformation> {
    let message = build_message("GetServerInformation");
    let connection = Connection::get_private(BusType::Session)?;
    let reply = connection.send_with_reply_and_block(message, 2000)?;

    let items = reply.get_items();

    Ok(ServerInformation {
        name:         unwrap_message_string(items.get(0)),
        vendor:       unwrap_message_string(items.get(1)),
        version:      unwrap_message_string(items.get(2)),
        spec_version: unwrap_message_string(items.get(3)) })
}

/// Return value of `get_server_information()`.
#[derive(Debug)]
pub struct ServerInformation {
    /// The product name of the server.
    pub name: String,
    /// The vendor name.
    pub vendor: String,
    /// The server's version string.
    pub version: String,
    /// The specification version the server is compliant with.
    pub spec_version: String
}

/// Strictly internal.
/// The NotificationServer implemented here exposes a "Stop" function.
/// stops the notification server
#[cfg(all(feature = "server", unix, not(target_os = "macos")))]
#[doc(hidden)]
pub fn stop_server() {
    let message = build_message("Stop");
    let connection = Connection::get_private(BusType::Session).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    connection.send(message).unwrap();
}



/// Listens for the `ActionInvoked(UInt32, String)` Signal.
///
/// No need to use this, check out `Notification::show_and_wait_for_action(FnOnce(action:&str))`
pub fn handle_action<F>(id: u32, func: F)
    where F: FnOnce(&str)
{
    let connection = Connection::get_private(BusType::Session).unwrap();
    wait_for_action_signal(&connection, id, func);
}



// here be non public functions

// Listens for the `ActionInvoked(UInt32, String)` signal.
fn wait_for_action_signal<F>(connection: &Connection, id: u32, func: F)
    where F: FnOnce(&str)
{
    connection.add_match("interface='org.freedesktop.Notifications',member='ActionInvoked'")
              .unwrap();
    connection.add_match("interface='org.freedesktop.Notifications',member='ActionInvoked'")
              .unwrap();
    connection.add_match("interface='org.freedesktop.Notifications',member='NotificationClosed'")
              .unwrap();

    for item in connection.iter(1000) {
        if let ConnectionItem::Signal(message) = item {
            let items = message.get_items();

            let (path, interface, member) = (
                message.path()     .map(|p| p.as_cstr().to_string_lossy().into_owned()).unwrap_or_else(String::new),
                message.interface().map(|p| p.as_cstr().to_string_lossy().into_owned()).unwrap_or_else(String::new),
                message.member()   .map(|p| p.as_cstr().to_string_lossy().into_owned()).unwrap_or_else(String::new)
            );
            match (path.as_ref(), interface.as_ref(), member.as_ref()) {
            // match (protocol.unwrap(), iface.unwrap(), member.unwrap()) {
                // Action Invoked
                ("/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked") => {
                    if let (&MessageItem::UInt32(nid), &MessageItem::Str(ref action)) = (&items[0], &items[1]) {
                        if nid == id {
                            func(action);
                            break;
                        }
                    }
                }

                // Notification Closed
                ("/org/freedesktop/Notifications", "org.freedesktop.Notifications", "NotificationClosed") => {
                    if let (&MessageItem::UInt32(nid), &MessageItem::UInt32(_)) = (&items[0], &items[1]) {
                        if nid == id {
                            func("__closed");
                            break;
                        }
                    }
                }
                (..) => ()
            }
        }
    }
}

pub fn build_message(method_name: &str) -> Message {
    Message::new_method_call(NOTIFICATION_NAMESPACE,
                             NOTIFICATION_OBJECTPATH,
                             NOTIFICATION_NAMESPACE,
                             method_name)
        .unwrap_or_else(|_| panic!("Error building message call {:?}.", method_name))
}

fn unwrap_message_string(item: Option<&MessageItem>) -> String {
    match item {
        Some(&MessageItem::Str(ref value)) => value.to_owned(),
        _ => "".to_owned()
    }
}
