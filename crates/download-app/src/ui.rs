use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    SetSaveDir(String),
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
    banner: Option<String>,
    model: SharedModel,
    commands: mpsc::UnboundedSender<Command>,
    server: tokio::task::JoinHandle<()>,
    aria2: Option<Child>,
}

impl DownloadUi {
    pub fn new(
        save_dir: String,
        banner: Option<String>,
        model: SharedModel,
        commands: mpsc::UnboundedSender<Command>,
        server: tokio::task::JoinHandle<()>,
        aria2: Option<Child>,
    ) -> Self {
        Self {
            url: String::new(),
            file_name: String::new(),
            save_dir,
            banner,
            model,
            commands,
            server,
            aria2,
        }
    }

    fn submit_add(&mut self) {
        let url = self.url.trim().to_string();
        let file_name = self.file_name.trim().to_string();
        let save_dir = self.save_dir.trim().to_string();
        if let Err(err) = validate_task_input(&url, &file_name) {
            self.model.set_notice(Some(err));
            return;
        }
        if save_dir.is_empty() {
            self.model
                .set_notice(Some("save directory required".into()));
            return;
        }
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
                    let commit_save = save.lost_focus()
                        || (save.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    if commit_save {
                        self.save_dir = self.save_dir.trim().to_string();
                        self.send(Command::SetSaveDir(self.save_dir.clone()));
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
}

impl ErrorScreen {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
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

pub fn run(app: DownloadUi) -> eframe::Result<()> {
    eframe::run_native(
        "Download Tool",
        native_options("Download Tool", [980.0, 680.0]),
        Box::new(|_cc| Ok(Box::new(app))),
    )
}

pub fn run_error(message: &str) -> eframe::Result<()> {
    let message = message.to_string();
    eframe::run_native(
        "Download Tool",
        native_options("Download Tool", [560.0, 240.0]),
        Box::new(move |_cc| Ok(Box::new(ErrorScreen::new(message)))),
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
    use super::{can_cancel, format_bytes, validate_task_input};
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
}
