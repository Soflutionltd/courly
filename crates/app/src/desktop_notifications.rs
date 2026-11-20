//! Desktop notifications through the macOS notification center.
//!
//! macOS shows them only while Courly is not the frontmost app, and only once
//! the user allowed notifications for it. A click reaches the app as the
//! notification's identifier on the [`clicks`] subscription.

use iced::futures::SinkExt;

use crate::message::Message;

/// One banner to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesktopNotification {
    /// Stable per message so a repeat replaces the earlier banner.
    pub identifier: String,
    /// Groups banners of one conversation in the notification center.
    pub group: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::Mutex;

    use block2::RcBlock;
    use iced::futures::channel::mpsc::UnboundedSender;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread, define_class, msg_send};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };

    use super::DesktopNotification;

    static CLICK_SENDER: Mutex<Option<UnboundedSender<String>>> = Mutex::new(None);

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and the class
        // does not implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "CourlyNotificationDelegate"]
        struct ClickDelegate;

        unsafe impl NSObjectProtocol for ClickDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for ClickDelegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion_handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let identifier = response.notification().request().identifier().to_string();
                if let Ok(sender) = CLICK_SENDER.lock()
                    && let Some(sender) = sender.as_ref()
                {
                    // The receiver only goes away with the subscription.
                    let _ = sender.unbounded_send(identifier);
                }
                completion_handler.call(());
            }
        }
    );

    impl ClickDelegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: NSObject's designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The notification center, or `None` for a binary run outside an app
    /// bundle, where asking for the center raises an Objective-C exception.
    fn center() -> Option<Retained<UNUserNotificationCenter>> {
        NSBundle::mainBundle().bundleIdentifier()?;
        Some(UNUserNotificationCenter::currentNotificationCenter())
    }

    pub(super) fn request_authorization() {
        let Some(center) = center() else {
            log::info!("desktop notifications need the app bundle; skipped");
            return;
        };
        let handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
            if !error.is_null() {
                // SAFETY: a non-null error is a valid NSError for this call.
                let error = unsafe { &*error };
                log::warn!("notification authorization failed: {error}");
            } else if !granted.as_bool() {
                log::info!("notifications are turned off for Courly in System Settings");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &handler,
        );
    }

    pub(super) fn post(notifications: &[DesktopNotification]) {
        let Some(center) = center() else {
            return;
        };
        for notification in notifications {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&notification.title));
            content.setSubtitle(&NSString::from_str(&notification.subtitle));
            content.setBody(&NSString::from_str(&notification.body));
            content.setThreadIdentifier(&NSString::from_str(&notification.group));
            content.setSound(Some(&UNNotificationSound::defaultSound()));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&notification.identifier),
                &content,
                None,
            );
            let handler = RcBlock::new(|error: *mut NSError| {
                if !error.is_null() {
                    // SAFETY: a non-null error is a valid NSError for this call.
                    let error = unsafe { &*error };
                    log::warn!("desktop notification not shown: {error}");
                }
            });
            center.addNotificationRequest_withCompletionHandler(&request, Some(&handler));
        }
    }

    pub(super) fn listen(sender: UnboundedSender<String>) {
        if let Ok(mut slot) = CLICK_SENDER.lock() {
            *slot = Some(sender);
        }
        let Some(center) = center() else {
            return;
        };
        let delegate = ClickDelegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds its delegate weakly; this one lives as long as
        // the process.
        let _ = Retained::into_raw(delegate);
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use iced::futures::channel::mpsc::UnboundedSender;

    use super::DesktopNotification;

    pub(super) fn request_authorization() {}

    pub(super) fn post(_notifications: &[DesktopNotification]) {}

    pub(super) fn listen(_sender: UnboundedSender<String>) {}
}

/// Ask macOS for permission to show notifications. The system prompts only
/// the first time; later calls return the stored answer.
pub(crate) fn request_authorization() {
    platform::request_authorization();
}

pub(crate) fn post(notifications: &[DesktopNotification]) {
    platform::post(notifications);
}

/// Clicked notifications, as `Message::DesktopNotificationClicked` with the
/// notification's identifier.
pub(crate) fn clicks() -> iced::Subscription<Message> {
    iced::Subscription::run(|| {
        iced::stream::channel(32, async |mut output| {
            let (sender, mut receiver) = iced::futures::channel::mpsc::unbounded();
            platform::listen(sender);
            while let Some(identifier) = iced::futures::StreamExt::next(&mut receiver).await {
                if output
                    .send(Message::DesktopNotificationClicked(identifier))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    })
}
