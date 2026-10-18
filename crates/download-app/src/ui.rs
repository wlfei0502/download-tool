use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use download_core::settings::Settings;
use download_core::task::{require_http_url, DownloadTask, TaskId, TaskStatus};
use download_local_api::DEFAULT_PORT;
use tokio::sync::mpsc;

pub enum Command {
    Add {
        url: String,
        file_name: String,
        save_dir: String,
    },
    Pause(TaskId),
    Resume(TaskId),
    Cancel(TaskId),
    PauseAll,
    UpdateSettings(Settings),
}

#[derive(Default)]
struct ModelInner {
    tasks: Vec<DownloadTask>,
    notice: Option<String>,
}

/// UI-facing copy of the shared task list and the latest action message.
#[derive(Clone)]
pub struct SharedModel {
    inner: Arc<Mutex<ModelInner>>,
}

impl SharedModel {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ModelInner::default())),
        }
    }

    pub fn set_notice(&self, notice: Option<String>) {
        self.lock().notice = notice;
    }

    pub fn set_tasks(&self, mut tasks: Vec<DownloadTask>) {
        tasks.sort_by(|left, right| {
            left.file_name
                .cmp(&right.file_name)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        self.lock().tasks = tasks;
    }

    pub fn view(&self) -> (Vec<DownloadTask>, Option<String>) {
        let inner = self.lock();
        (inner.tasks.clone(), inner.notice.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ModelInner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}

pub struct DownloadUi {
    url: String,
    file_name: String,
    save_dir: String,
    speed_limit: String,
    max_connections: String,
    autostart: bool,
    banner: Option<String>,
    model: SharedModel,
    commands: mpsc::UnboundedSender<Command>,
    server: tokio::task::JoinHandle<()>,
    aria2: Option<Child>,
    tray: Option<crate::tray::Tray>,
    quit: bool,
    activation: crate::single_instance::Activation,
    /// Keeps the process lock until the window loop ends.
    #[allow(dead_code)]
    instance: Option<crate::single_instance::InstanceLock>,
}

impl DownloadUi {
    pub fn new(
        settings: Settings,
        banner: Option<String>,
        model: SharedModel,
        commands: mpsc::UnboundedSender<Command>,
        server: tokio::task::JoinHandle<()>,
        aria2: Option<Child>,
        activation: crate::single_instance::Activation,
    ) -> Self {
        Self {
            url: String::new(),
            file_name: String::new(),
            speed_limit: format_speed_limit_kbps(settings.global_speed_limit_bps),
            max_connections: settings.max_connections_per_server.to_string(),
            autostart: settings.autostart,
            save_dir: settings.save_dir,
            banner,
            model,
            commands,
            server,
            aria2,
            tray: None,
            quit: false,
            activation,
            instance: None,
        }
    }

    pub fn hold_instance(&mut self, lock: crate::single_instance::InstanceLock) {
        self.instance = Some(lock);
    }

    fn apply_activation(&mut self, ctx: &egui::Context) {
        self.activation.bind_context(ctx);
        if self.activation.take_show() {
            show_main_window(ctx);
        }
    }

    fn install_tray(&mut self, ctx: &egui::Context) {
        match crate::tray::Tray::install(ctx) {
            Ok(tray) => self.tray = Some(tray),
            Err(err) => eprintln!("system tray unavailable: {err}"),
        }
    }

    fn apply_tray(&mut self, ctx: &egui::Context) {
        let actions = self
            .tray
            .as_ref()
            .map(|tray| tray.poll())
            .unwrap_or_default();
        for action in actions {
            match action {
                crate::tray::TrayAction::Show => show_main_window(ctx),
                crate::tray::TrayAction::PauseAll => self.send(Command::PauseAll),
                crate::tray::TrayAction::Quit => {
                    self.quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn handle_window_close(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && crate::tray::hide_on_close(self.tray.is_some(), self.quit) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn submit_add(&mut self) {
        let url = self.url.trim().to_string();
        let file_name = self.file_name.trim().to_string();
        if let Err(err) = validate_task_input(&url, &file_name) {
            self.model.set_notice(Some(err));
            return;
        }
        if !self.commit_settings() {
            return;
        }
        let save_dir = self.save_dir.clone();
        if self
            .commands
            .send(Command::Add {
                url,
                file_name,
                save_dir,
            })
            .is_err()
        {
            self.model
                .set_notice(Some("download engine has stopped".into()));
            return;
        }
        self.url.clear();
        self.file_name.clear();
    }

    /// Persists the settings fields. Returns false when a field is invalid.
    fn commit_settings(&mut self) -> bool {
        let save_dir = self.save_dir.trim().to_string();
        if save_dir.is_empty() {
            self.model
                .set_notice(Some("save directory required".into()));
            return false;
        }
        let global_speed_limit_bps = match parse_speed_limit_kbps(&self.speed_limit) {
            Ok(limit) => limit,
            Err(err) => {
                self.model.set_notice(Some(err));
                return false;
            }
        };
        let max_connections_per_server = match parse_max_connections(&self.max_connections) {
            Ok(value) => value,
            Err(err) => {
                self.model.set_notice(Some(err));
                return false;
            }
        };
        self.save_dir = save_dir.clone();
        self.send(Command::UpdateSettings(Settings {
            save_dir,
            global_speed_limit_bps,
            max_connections_per_server,
            autostart: self.autostart,
        }));
        true
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            self.model
                .set_notice(Some("download engine has stopped".into()));
        }
    }
}

impl Drop for DownloadUi {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(child) = self.aria2.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl eframe::App for DownloadUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(400));
        self.apply_activation(ctx);
        self.apply_tray(ctx);
        self.handle_window_close(ctx);
        let (tasks, notice) = self.model.view();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Download Tool");
            ui.add_space(6.0);
            if let Some(banner) = &self.banner {
                ui.colored_label(egui::Color32::from_rgb(255, 186, 92), banner);
                ui.add_space(8.0);
            }

            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Save directory");
                    let save = ui.add(
                        egui::TextEdit::singleline(&mut self.save_dir)
                            .desired_width(520.0)
                            .hint_text("/path/to/downloads"),
                    );
                    if field_committed(&save, ui) {
                        self.commit_settings();
                    }
                    ui.end_row();

                    ui.label("Speed limit (KB/s)");
                    let speed = ui.add(
                        egui::TextEdit::singleline(&mut self.speed_limit)
                            .desired_width(160.0)
                            .hint_text("empty = unlimited"),
                    );
                    if field_committed(&speed, ui) {
                        self.commit_settings();
                    }
                    ui.end_row();

                    ui.label("Connections per server");
                    let connections = ui.add(
                        egui::TextEdit::singleline(&mut self.max_connections)
                            .desired_width(80.0)
                            .hint_text("1–16"),
                    );
                    if field_committed(&connections, ui) {
                        self.commit_settings();
                    }
                    ui.end_row();

                    ui.label("Start at login");
                    let previous_autostart = self.autostart;
                    if ui
                        .checkbox(&mut self.autostart, "Open when I log in")
                        .changed()
                        && !self.commit_settings()
                    {
                        self.autostart = previous_autostart;
                    }
                    ui.end_row();

                    ui.label("URL");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.url)
                            .desired_width(520.0)
                            .hint_text("http://example.com/file.zip"),
                    );
                    ui.end_row();

                    ui.label("File name");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.file_name)
                                .desired_width(420.0)
                                .hint_text("file.zip"),
                        );
                        if ui.button("Add").clicked() {
                            self.submit_add();
                        }
                    });
                    ui.end_row();
                });

            if let Some(notice) = notice {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(255, 140, 120), notice);
            }

            ui.add_space(8.0);
            ui.separator();
            ui.heading("Tasks");
            ui.label(format!("Local API  http://127.0.0.1:{DEFAULT_PORT}"));
            ui.add_space(4.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                if tasks.is_empty() {
                    ui.label("No downloads yet.");
                    return;
                }
                task_table(ui, &tasks, |command| self.send(command));
            });
        });
    }
}

fn task_table(ui: &mut egui::Ui, tasks: &[DownloadTask], mut send: impl FnMut(Command)) {
    egui::Grid::new("tasks")
        .striped(true)
        .num_columns(5)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.strong("File");
            ui.strong("Status");
            ui.strong("Progress");
            ui.strong("Speed");
            ui.strong("Actions");
            ui.end_row();

            for task in tasks {
                ui.label(&task.file_name).on_hover_text(&task.url);
                ui.vertical(|ui| {
                    ui.label(status_name(task.status));
                    if let Some(err) = &task.error {
                        ui.label(
                            egui::RichText::new(err)
                                .small()
                                .color(egui::Color32::from_rgb(255, 140, 120)),
                        );
                    }
                });
                progress_cell(ui, task);
                ui.label(speed_text(task));
                ui.push_id(&task.id.0, |ui| {
                    ui.horizontal(|ui| {
                        let pause = task.status == TaskStatus::Active;
                        let resume = task.status == TaskStatus::Paused;
                        let cancel = can_cancel(task);
                        if ui.add_enabled(pause, egui::Button::new("Pause")).clicked() {
                            send(Command::Pause(task.id.clone()));
                        }
                        if ui
                            .add_enabled(resume, egui::Button::new("Resume"))
                            .clicked()
                        {
                            send(Command::Resume(task.id.clone()));
                        }
                        if ui
                            .add_enabled(cancel, egui::Button::new("Cancel"))
                            .clicked()
                        {
                            send(Command::Cancel(task.id.clone()));
                        }
                    });
                });
                ui.end_row();
            }
        });
}

fn progress_cell(ui: &mut egui::Ui, task: &DownloadTask) {
    if let Some(total) = task.size.filter(|total| *total > 0) {
        let fraction = (task.completed_length as f32 / total as f32).clamp(0.0, 1.0);
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_width(180.0)
                .show_percentage(),
        );
        return;
    }
    if task.completed_length > 0 || task.status == TaskStatus::Complete {
        ui.label(format_bytes(task.completed_length));
    } else {
        ui.label("—");
    }
}

fn status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Queued => "Queued",
        TaskStatus::Active => "Active",
        TaskStatus::Paused => "Paused",
        TaskStatus::Complete => "Complete",
        TaskStatus::Error => "Error",
    }
}

fn speed_text(task: &DownloadTask) -> String {
    if task.status == TaskStatus::Active {
        format!("{}/s", format_bytes(task.download_speed))
    } else {
        "—".to_string()
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn field_committed(response: &egui::Response, ui: &egui::Ui) -> bool {
    response.lost_focus()
        || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
}

/// Blank or `0` means unlimited. Other values are kibibytes per second, stored as bytes.
pub fn parse_speed_limit_kbps(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("speed limit must be a whole number of KB/s".into());
    }
    let kbps: u64 = text
        .parse()
        .map_err(|_| "speed limit must be a whole number of KB/s".to_string())?;
    if kbps == 0 {
        return Ok(None);
    }
    kbps.checked_mul(1024)
        .map(Some)
        .ok_or_else(|| "speed limit is too large".into())
}

pub fn format_speed_limit_kbps(limit_bps: Option<u64>) -> String {
    match limit_bps {
        None | Some(0) => String::new(),
        Some(bps) => (bps / 1024).to_string(),
    }
}

pub fn parse_max_connections(text: &str) -> Result<u8, String> {
    let text = text.trim();
    let value: u16 = if text.is_empty() {
        0
    } else if text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse().unwrap_or(0)
    } else {
        0
    };
    if (1..=16).contains(&value) {
        Ok(value as u8)
    } else {
        Err("connections per server must be from 1 to 16".into())
    }
}

pub fn can_cancel(task: &DownloadTask) -> bool {
    if task.gid.is_none() {
        return matches!(task.status, TaskStatus::Queued | TaskStatus::Error);
    }
    matches!(task.status, TaskStatus::Active | TaskStatus::Paused)
}

pub fn validate_task_input(url: &str, file_name: &str) -> Result<(), String> {
    require_http_url(url)?;
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return Err("file name required".into());
    }
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains('\0') {
        return Err("file name must not include a path".into());
    }
    Ok(())
}

pub struct ErrorScreen {
    message: String,
    /// Keeps the process lock until the error window closes.
    #[allow(dead_code)]
    instance: crate::single_instance::InstanceLock,
}

impl ErrorScreen {
    pub fn new(message: impl Into<String>, instance: crate::single_instance::InstanceLock) -> Self {
        Self {
            message: message.into(),
            instance,
        }
    }
}

impl eframe::App for ErrorScreen {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Download Tool could not start");
            ui.add_space(12.0);
            ui.label(&self.message);
            ui.add_space(16.0);
            if ui.button("Quit").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

pub fn run(mut app: DownloadUi) -> eframe::Result<()> {
    eframe::run_native(
        "Download Tool",
        native_options("Download Tool", [980.0, 680.0]),
        Box::new(move |cc| {
            // The winit loop is already running here, which tray-icon requires
            // before a status item is created (especially on macOS).
            app.install_tray(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

fn show_main_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

pub fn run_error(
    message: &str,
    instance: crate::single_instance::InstanceLock,
) -> eframe::Result<()> {
    let message = message.to_string();
    eframe::run_native(
        "Download Tool",
        native_options("Download Tool", [560.0, 240.0]),
        Box::new(move |_cc| Ok(Box::new(ErrorScreen::new(message, instance)))),
    )
}

fn native_options(title: &str, size: [f32; 2]) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(size)
            .with_min_inner_size([480.0, 280.0]),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        can_cancel, format_bytes, format_speed_limit_kbps, parse_max_connections,
        parse_speed_limit_kbps, validate_task_input,
    };
    use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};

    fn task(status: TaskStatus, gid: Option<&str>) -> DownloadTask {
        let mut task = DownloadTask::from_request(NewTaskRequest {
            url: "http://example.com/a.zip".into(),
            file_name: "a.zip".into(),
            size: None,
            referrer: None,
            user_agent: None,
        });
        task.status = status;
        task.gid = gid.map(str::to_string);
        task
    }

    #[test]
    fn rejects_empty_url_and_path_file_names() {
        assert_eq!(
            validate_task_input("", "a.zip").unwrap_err(),
            "url required"
        );
        assert!(validate_task_input("http://example.com/a", "").is_err());
        assert!(validate_task_input("http://example.com/a", "../secret").is_err());
        assert!(validate_task_input("http://example.com/a", "a/b.zip").is_err());
        assert!(validate_task_input("http://example.com/a", "a.zip").is_ok());
    }

    #[test]
    fn rejects_non_http_urls_and_accepts_http() {
        assert_eq!(
            validate_task_input("https://example.com/a", "a.zip").unwrap_err(),
            "only http:// URLs are allowed"
        );
        assert_eq!(
            validate_task_input("HTTPS://example.com/a", "a.zip").unwrap_err(),
            "only http:// URLs are allowed"
        );
        assert_eq!(
            validate_task_input("ftp://example.com/a", "a.zip").unwrap_err(),
            "only http:// URLs are allowed"
        );
        assert!(validate_task_input("http://example.com/a", "a.zip").is_ok());
        assert!(validate_task_input("HTTP://example.com/a", "a.zip").is_ok());
        assert_eq!(
            validate_task_input("http://user:pass@example.com/a", "a.zip").unwrap_err(),
            "url must not include user info"
        );
    }

    #[test]
    fn cancel_enabled_for_queued_and_error_without_gid() {
        assert!(can_cancel(&task(TaskStatus::Queued, None)));
        assert!(can_cancel(&task(TaskStatus::Error, None)));
        assert!(can_cancel(&task(TaskStatus::Active, Some("gid-1"))));
        assert!(can_cancel(&task(TaskStatus::Paused, Some("gid-1"))));
        assert!(!can_cancel(&task(TaskStatus::Active, None)));
        assert!(!can_cancel(&task(TaskStatus::Complete, None)));
        assert!(!can_cancel(&task(TaskStatus::Error, Some("gid-1"))));
    }

    #[test]
    fn formats_byte_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn speed_limit_field_uses_kibibytes_and_blank_means_unlimited() {
        assert_eq!(parse_speed_limit_kbps("").unwrap(), None);
        assert_eq!(parse_speed_limit_kbps("  0 ").unwrap(), None);
        assert_eq!(parse_speed_limit_kbps("1024").unwrap(), Some(1024 * 1024));
        assert!(parse_speed_limit_kbps("1.5").is_err());
        assert!(parse_speed_limit_kbps("-1").is_err());
        assert_eq!(format_speed_limit_kbps(None), "");
        assert_eq!(format_speed_limit_kbps(Some(2048)), "2");
    }

    #[test]
    fn connections_must_be_from_1_to_16() {
        assert_eq!(parse_max_connections("8").unwrap(), 8);
        assert_eq!(parse_max_connections(" 16 ").unwrap(), 16);
        assert!(parse_max_connections("0").is_err());
        assert!(parse_max_connections("17").is_err());
        assert!(parse_max_connections("").is_err());
    }
}
