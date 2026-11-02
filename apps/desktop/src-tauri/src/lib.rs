//! Trading Center desktop shell.
//!
//! Security model (docs/decisions/proposals/security.md §5):
//! * The window loads the remote HTTPS origin (`TC_ORIGIN`, fixed at build time). Passkeys
//!   (Windows Hello) are bound to that origin; a bundled `tauri.localhost` UI could not use them.
//! * The remote page gets NO IPC: no capability grants remote URLs anything, and this crate
//!   registers no commands. The tray menu is driven entirely from Rust.
//! * Navigation is pinned to the configured origin; anything else is blocked.
//! * Single instance: a second launch focuses the existing window.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;

const DEFAULT_ORIGIN: &str = "https://trading.example.com";

fn origin() -> Url {
    let raw = option_env!("TC_ORIGIN").unwrap_or(DEFAULT_ORIGIN);
    let url = Url::parse(raw).unwrap_or_else(|_| Url::parse(DEFAULT_ORIGIN).expect("valid default origin"));
    // Only HTTPS is accepted (passkeys and Secure cookies require it); localhost allowed for development.
    let local = matches!(url.host_str(), Some("localhost") | Some("127.0.0.1"));
    if url.scheme() == "https" || local {
        url
    } else {
        Url::parse(DEFAULT_ORIGIN).expect("valid default origin")
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let home = origin();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main(app)))
        .setup(move |app| {
            let allowed = home.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(home.clone()))
                .title("Trading Center")
                .inner_size(1280.0, 820.0)
                .min_inner_size(380.0, 600.0)
                .background_color(tauri::window::Color(0, 0, 0, 255))
                .theme(Some(tauri::Theme::Dark))
                .on_navigation(move |url| same_origin(url, &allowed))
                .build()?;

            let open = MenuItem::with_id(app, "open", "Megnyitás", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Kilépés", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Trading Center")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Trading Center desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_pinned_to_origin() {
        let home = Url::parse("https://trading.example.com/").unwrap();
        assert!(same_origin(&Url::parse("https://trading.example.com/positions/").unwrap(), &home));
        assert!(!same_origin(&Url::parse("https://evil.example.com/").unwrap(), &home));
        assert!(!same_origin(&Url::parse("http://trading.example.com/").unwrap(), &home));
        assert!(!same_origin(&Url::parse("https://trading.example.com:8443/").unwrap(), &home));
    }

    #[test]
    fn default_origin_is_https() {
        assert_eq!(origin().scheme(), "https");
    }
}
