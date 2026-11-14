//! Living in the background on Windows: one running copy at a time, an icon
//! in the notification area, starting with Windows, and toast notifications.
//! Everything outside the window reaches the app as a `Wake` on `events()`.

use iced::futures::channel::mpsc;
use std::sync::Mutex;

/// Something outside the window asking the app to act.
#[derive(Debug, Clone)]
pub enum Wake {
    /// The tray icon was clicked, or GUC Station was opened again.
    Show,
    /// A toast was clicked.
    Open(Target),
    Quit,
}

/// What a toast is about, so clicking it opens the right screen.
#[derive(Debug, Clone)]
pub enum Target {
    Mail(String),
    Notifications,
}

static SINK: Mutex<Option<mpsc::UnboundedSender<Wake>>> = Mutex::new(None);

fn send(w: Wake) {
    if let Some(tx) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.unbounded_send(w);
    }
}

/// The stream behind the app's subscription.
pub fn events() -> mpsc::UnboundedReceiver<Wake> {
    let (tx, rx) = mpsc::unbounded();
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    rx
}

/// Passed by the Windows startup entry, so the app opens straight to the tray.
const BACKGROUND_ARG: &str = "--background";

pub fn started_in_background() -> bool {
    std::env::args().any(|a| a == BACKGROUND_ARG)
}

#[cfg(windows)]
pub use win::{acknowledge_show, claim_instance, notify, set_start_on_boot, tray};

#[cfg(not(windows))]
mod other {
    pub fn claim_instance() -> bool {
        true
    }
    pub fn acknowledge_show() {}
    pub fn tray() -> Option<()> {
        None
    }
    pub fn set_start_on_boot(_: bool) {}
    pub fn notify(_: Vec<(String, String, super::Target)>) {}
}
#[cfg(not(windows))]
pub use other::*;

#[cfg(windows)]
mod win {
    use super::{BACKGROUND_ARG, Target, Wake, send};
    use std::sync::{Mutex, OnceLock};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
    use windows::Win32::System::Threading::{
        CreateEventW, INFINITE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent, WaitForSingleObject,
    };
    use windows::core::{HSTRING, PCWSTR, w};

    /// Set by a second copy to ask this one to show its window.
    const SHOW: PCWSTR = w!("Local\\GUCStation.Show");
    /// Set by this copy once it has shown its window.
    const SHOWN: PCWSTR = w!("Local\\GUCStation.Shown");
    const ERROR_ALREADY_EXISTS: i32 = 183;

    static SHOWN_EVENT: OnceLock<isize> = OnceLock::new();

    /// True when this is the only copy running. Otherwise asks the running
    /// copy to show itself and returns false once it has. A copy that is just
    /// closing (as it does after installing an update) never answers, so this
    /// keeps waiting for it to go away and then takes over.
    pub fn claim_instance() -> bool {
        for attempt in 0..40 {
            let Ok(show) = (unsafe { CreateEventW(None, false, false, SHOW) }) else { return true };
            let existed = std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ALREADY_EXISTS);
            if !existed {
                let shown = unsafe { CreateEventW(None, false, false, SHOWN) }.map(|h| h.0).unwrap_or(0);
                let _ = SHOWN_EVENT.set(shown);
                let raw = show.0;
                std::thread::spawn(move || {
                    loop {
                        if unsafe { WaitForSingleObject(HANDLE(raw), INFINITE) } != WAIT_OBJECT_0 {
                            return;
                        }
                        send(Wake::Show);
                    }
                });
                return true;
            }
            if attempt == 0 {
                let _ = unsafe { SetEvent(show) };
            }
            let _ = unsafe { CloseHandle(show) };
            match unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, SHOWN) } {
                Ok(ack) => {
                    let answered = unsafe { WaitForSingleObject(ack, 300) } == WAIT_OBJECT_0;
                    let _ = unsafe { CloseHandle(ack) };
                    if answered {
                        return false;
                    }
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(300)),
            }
        }
        true
    }

    /// Tells a second copy waiting in `claim_instance` that the window is up.
    pub fn acknowledge_show() {
        if let Some(&h) = SHOWN_EVENT.get().filter(|h| **h != 0) {
            let _ = unsafe { SetEvent(HANDLE(h)) };
        }
    }

    /// The notification-area icon. It disappears when dropped, so keep it for
    /// as long as the app runs. Left click opens the window; right click
    /// shows a menu with Open and Quit.
    pub fn tray() -> Option<TrayIcon> {
        let png = image::load_from_memory(include_bytes!("../assets/brand/app-icon-256.png")).ok()?;
        let rgba = png.resize(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
        let (width, height) = rgba.dimensions();
        let icon = tray_icon::Icon::from_rgba(rgba.into_raw(), width, height).ok()?;

        let menu = Menu::new();
        let _ = menu.append(&MenuItem::with_id("open", "Open GUC Station", true, None));
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id("quit", "Quit", true, None));

        TrayIconEvent::set_event_handler(Some(|e| match e {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
            | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => send(Wake::Show),
            _ => {}
        }));
        MenuEvent::set_event_handler(Some(|e: MenuEvent| match e.id.0.as_str() {
            "open" => send(Wake::Show),
            "quit" => send(Wake::Quit),
            _ => {}
        }));

        TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("GUC Station")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn set_string(key: &str, name: &str, value: &str) -> windows::core::Result<()> {
        let (key, name, data) = (wide(key), wide(name), wide(value));
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        }
    }

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUN_VALUE: &str = "GUC Station";

    /// Adds or removes the entry that starts GUC Station when you sign in to
    /// Windows. Debug builds never add it, so a development copy doesn't
    /// start itself on boot.
    pub fn set_start_on_boot(enabled: bool) {
        if enabled && !cfg!(debug_assertions) {
            if let Ok(exe) = std::env::current_exe() {
                let _ = set_string(RUN_KEY, RUN_VALUE, &format!("\"{}\" {BACKGROUND_ARG}", exe.display()));
            }
        } else {
            let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
            let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr())) };
        }
    }

    /// Toasts need an app identity. Unpackaged apps get one by registering
    /// a name and icon under this key.
    const AUMID: &str = "GUCStation.App";

    fn register_identity() -> bool {
        static DONE: OnceLock<bool> = OnceLock::new();
        *DONE.get_or_init(|| {
            let icon = crate::settings::data_dir().join("notification-icon.png");
            let _ = std::fs::create_dir_all(icon.parent().unwrap_or(&icon));
            let _ = std::fs::write(&icon, include_bytes!("../assets/brand/app-icon-256.png"));
            let key = format!("Software\\Classes\\AppUserModelId\\{AUMID}");
            set_string(&key, "DisplayName", "GUC Station").is_ok() && set_string(&key, "IconUri", &icon.display().to_string()).is_ok()
        })
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    /// Toasts have to outlive `notify` for their click handlers to run.
    static SHOWN_TOASTS: Mutex<Vec<ToastNotification>> = Mutex::new(Vec::new());

    fn show(title: &str, body: &str, target: Target) -> windows::core::Result<()> {
        let xml = format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
            escape(title),
            escape(body)
        );
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            send(Wake::Open(target.clone()));
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)?;
        let mut kept = SHOWN_TOASTS.lock().unwrap_or_else(|e| e.into_inner());
        kept.push(toast);
        let extra = kept.len().saturating_sub(30);
        kept.drain(..extra);
        Ok(())
    }

    /// Shows a toast for each (title, body, target), off the UI thread.
    pub fn notify(toasts: Vec<(String, String, Target)>) {
        if toasts.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            // Keeps a process-wide multithreaded apartment alive for WinRT, as the PDF viewer does.
            let _ = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() };
            if !register_identity() {
                return;
            }
            for (title, body, target) in toasts {
                let _ = show(&title, &body, target);
            }
        });
    }
}
