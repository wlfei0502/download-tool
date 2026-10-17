//! Status-item menu: Show, Pause all, and Quit.
//!
//! The icon is created on the UI thread after eframe's event loop is running.
//! Menu clicks are forwarded on a side thread so the egui loop wakes immediately.

use download_core::task::DownloadTask;
use download_core::task::TaskStatus;

pub const SHOW_ID: &str = "show";
pub const PAUSE_ALL_ID: &str = "pause-all";
pub const QUIT_ID: &str = "quit";

const ICON_SIZE: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    PauseAll,
    Quit,
}

pub struct Tray {
    /// Dropping the icon removes it from the status bar.
    #[allow(dead_code)]
    icon: tray_icon::TrayIcon,
    events: std::sync::mpsc::Receiver<TrayAction>,
}

impl Tray {
    /// Installs the status item. Call this on the UI thread once the winit
    /// loop is already running (eframe's app-creation callback).
    pub fn install(ctx: &egui::Context) -> Result<Self, String> {
        let (events_tx, events) = std::sync::mpsc::channel();
        let icon = build_icon()?;
        let menu = build_menu()?;
        let icon = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Download Tool")
            .with_icon(icon)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .build()
            .map_err(|err| err.to_string())?;

        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("tray-menu".into())
            .spawn(move || listen_for_menu(events_tx, ctx))
            .map_err(|err| format!("start tray menu listener: {err}"))?;

        Ok(Self { icon, events })
    }

    pub fn poll(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(action) = self.events.try_recv() {
            actions.push(action);
        }
        actions
    }
}

fn listen_for_menu(events_tx: std::sync::mpsc::Sender<TrayAction>, ctx: egui::Context) {
    let receiver = tray_icon::menu::MenuEvent::receiver();
    while let Ok(event) = receiver.recv() {
        let Some(action) = action_for_menu_id(event.id.as_ref()) else {
            continue;
        };
        if events_tx.send(action).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn build_menu() -> Result<tray_icon::menu::Menu, String> {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

    let menu = Menu::new();
    let show = MenuItem::with_id(SHOW_ID, "Show", true, None);
    let pause_all = MenuItem::with_id(PAUSE_ALL_ID, "Pause all", true, None);
    let quit = MenuItem::with_id(QUIT_ID, "Quit", true, None);
    let separator = PredefinedMenuItem::separator();
    menu.append_items(&[&show, &pause_all, &separator, &quit])
        .map_err(|err| err.to_string())?;
    Ok(menu)
}

fn build_icon() -> Result<tray_icon::Icon, String> {
    // macOS template images are black and clear; other platforms tint a blue arrow.
    let color = if cfg!(target_os = "macos") {
        [0, 0, 0, 255]
    } else {
        [37, 99, 235, 255]
    };
    let rgba = download_icon_rgba(ICON_SIZE, color);
    tray_icon::Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).map_err(|err| err.to_string())
}

pub fn action_for_menu_id(id: &str) -> Option<TrayAction> {
    match id {
        SHOW_ID => Some(TrayAction::Show),
        PAUSE_ALL_ID => Some(TrayAction::PauseAll),
        QUIT_ID => Some(TrayAction::Quit),
        _ => None,
    }
}

/// Closing the window hides it while the tray is installed.
/// Quit, or a tray that failed to install, lets the process exit so the
/// aria2 child is stopped with the window.
pub fn hide_on_close(tray_installed: bool, quit_requested: bool) -> bool {
    tray_installed && !quit_requested
}

/// Active downloads and not-yet-started queued tasks.
/// Finished, failed, and already paused tasks are left alone.
pub fn pause_all_ids(tasks: &[DownloadTask]) -> Vec<download_core::task::TaskId> {
    tasks
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::Active | TaskStatus::Queued))
        .map(|task| task.id.clone())
        .collect()
}

/// RGBA8 downward arrow. `color` is the opaque arrow; the rest is transparent.
pub fn download_icon_rgba(size: u32, color: [u8; 4]) -> Vec<u8> {
    let size = size.max(1);
    let mut pixels = vec![0u8; (size as usize) * (size as usize) * 4];
    let side = size as i32;
    let center_x = side / 2;
    let shaft_half = (side / 10).max(1);
    let shaft_top = side / 6;
    let shaft_bottom = side * 9 / 16;
    let head_top = side * 7 / 16;
    let head_bottom = side * 5 / 6;
    let head_half = (side * 3 / 8).max(shaft_half + 1);
    let head_span = (head_bottom - head_top).max(1);

    for y in 0..side {
        for x in 0..side {
            let in_shaft =
                (x - center_x).abs() <= shaft_half && y >= shaft_top && y <= shaft_bottom;
            let in_head = if y >= head_top && y <= head_bottom {
                let along = (y - head_top) as f32 / head_span as f32;
                let half = (head_half as f32 * (1.0 - along)).round() as i32;
                (x - center_x).abs() <= half
            } else {
                false
            };
            if in_shaft || in_head {
                let index = ((y as usize) * (size as usize) + (x as usize)) * 4;
                pixels[index..index + 4].copy_from_slice(&color);
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::{
        action_for_menu_id, download_icon_rgba, hide_on_close, pause_all_ids, PAUSE_ALL_ID,
        QUIT_ID, SHOW_ID,
    };
    use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};

    #[test]
    fn menu_ids_map_to_show_pause_all_and_quit() {
        assert_eq!(action_for_menu_id(SHOW_ID), Some(super::TrayAction::Show));
        assert_eq!(
            action_for_menu_id(PAUSE_ALL_ID),
            Some(super::TrayAction::PauseAll)
        );
        assert_eq!(action_for_menu_id(QUIT_ID), Some(super::TrayAction::Quit));
        assert_eq!(action_for_menu_id("other"), None);
    }

    #[test]
    fn close_hides_until_quit_or_missing_tray() {
        assert!(hide_on_close(true, false));
        assert!(!hide_on_close(true, true));
        assert!(!hide_on_close(false, false));
        assert!(!hide_on_close(false, true));
    }

    #[test]
    fn pause_all_selects_active_and_queued_only() {
        let queued = task(TaskStatus::Queued);
        let active = task(TaskStatus::Active);
        let paused = task(TaskStatus::Paused);
        let complete = task(TaskStatus::Complete);
        let error = task(TaskStatus::Error);
        let ids = pause_all_ids(&[queued.clone(), active.clone(), paused, complete, error]);
        assert_eq!(ids, vec![queued.id, active.id]);
    }

    #[test]
    fn download_icon_is_a_solid_arrow_on_a_clear_background() {
        let color = [37, 99, 235, 255];
        let size = 32u32;
        let rgba = download_icon_rgba(size, color);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        assert_eq!(pixel(&rgba, size, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&rgba, size, size - 1, size - 1), [0, 0, 0, 0]);
        assert!(rgba.chunks(4).any(|pixel| pixel == color));
        assert!(rgba
            .chunks(4)
            .all(|pixel| pixel == [0, 0, 0, 0] || pixel == color));
    }

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * size + x) * 4) as usize;
        rgba[index..index + 4].try_into().expect("pixel")
    }

    fn task(status: TaskStatus) -> DownloadTask {
        let mut task = DownloadTask::from_request(NewTaskRequest {
            url: format!("http://example.com/{status:?}"),
            file_name: "a.zip".into(),
            size: None,
            referrer: None,
            user_agent: None,
        });
        task.status = status;
        task
    }
}
