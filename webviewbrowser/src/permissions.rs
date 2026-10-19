//! Site permission prompts on Linux.
//!
//! WebView2 (Windows) and WKWebView (macOS) show their own prompts. WebKitGTK denies requests
//! that nobody handles, so on Linux the browser asks with a native dialog, like Firefox does.

use wry::{PermissionKind, PermissionResponse};

/// What a permission lets a site do, phrased for a prompt. `None` for kinds left to the engine.
pub fn describe(kind: &PermissionKind) -> Option<&'static str> {
    Some(match kind {
        PermissionKind::Microphone => "use your microphone",
        PermissionKind::Camera => "use your camera",
        PermissionKind::Geolocation => "know your location",
        PermissionKind::Notifications => "send you notifications",
        PermissionKind::DisplayCapture => "share your screen",
        _ => return None,
    })
}

/// The host name shown in prompts.
pub fn origin_label(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "This page".into())
}

/// Shows a blocking Allow/Block dialog and returns the answer.
pub fn ask(kind: PermissionKind, page_url: &str) -> PermissionResponse {
    use gtk::prelude::*;
    let Some(what) = describe(&kind) else {
        return PermissionResponse::Default;
    };
    let dialog = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::MODAL,
        gtk::MessageType::Question,
        gtk::ButtonsType::None,
        &format!("Allow {} to {what}?", origin_label(page_url)),
    );
    dialog.add_button("Block", gtk::ResponseType::Reject);
    dialog.add_button("Allow", gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Reject);
    let answer = dialog.run();
    dialog.close();
    if answer == gtk::ResponseType::Accept {
        PermissionResponse::Allow
    } else {
        PermissionResponse::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_prompted_kinds_only() {
        assert_eq!(describe(&PermissionKind::Camera), Some("use your camera"));
        assert_eq!(describe(&PermissionKind::ClipboardRead), None);
        assert_eq!(origin_label("https://meet.example.com/room"), "meet.example.com");
        assert_eq!(origin_label("not a url"), "This page");
    }
}
