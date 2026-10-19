use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use download_core::protocol::suggested_file_name;
use download_core::settings::{CloseAction, Settings};
use download_core::task::{require_http_url, DownloadTask, TaskId, TaskStatus};
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
    ClearAll,
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
    /// Last name filled from the URL; used so edits are kept when the URL changes.
    auto_file_name: String,
    save_dir: String,
    speed_limit: String,
    max_connections: String,
    banner: Option<String>,
    model: SharedModel,
    commands: mpsc::UnboundedSender<Command>,
    server: tokio::task::JoinHandle<()>,
    aria2: Option<Child>,
    tray: Option<crate::tray::Tray>,
    quit: bool,
    close_ask: bool,
    close_action: CloseAction,
    /// When set, the close confirmation dialog is open.
    close_dialog: Option<CloseDialogState>,
    activation: crate::single_instance::Activation,
    /// Keeps the process lock until the window loop ends.
    #[allow(dead_code)]
    instance: Option<crate::single_instance::InstanceLock>,
}

#[derive(Default)]
struct CloseDialogState {
    dont_ask: bool,
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
            auto_file_name: String::new(),
            speed_limit: format_speed_limit_kbps(settings.global_speed_limit_bps),
            max_connections: settings.max_connections_per_server.to_string(),
            save_dir: settings.save_dir,
            banner,
            model,
            commands,
            server,
            aria2,
            tray: None,
            quit: false,
            close_ask: settings.close_ask,
            close_action: settings.close_action,
            close_dialog: None,
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
                crate::tray::TrayAction::Quit => self.request_quit(ctx),
            }
        }
    }

    /// Stop aria2 / the local API, then close the window for real (not to tray).
    fn request_quit(&mut self, ctx: &egui::Context) {
        self.quit = true;
        self.shutdown_backends();
        // Restore first: a minimized viewport on Windows sometimes ignores Close.
        show_main_window(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    fn shutdown_backends(&mut self) {
        self.server.abort();
        if let Some(mut child) = self.aria2.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn handle_window_close(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if !close_requested {
            return;
        }
        match crate::tray::close_decision(
            self.tray.is_some(),
            self.quit,
            self.close_ask,
            self.close_action,
        ) {
            crate::tray::CloseDecision::Ask => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.close_dialog.is_none() {
                    self.close_dialog = Some(CloseDialogState::default());
                }
            }
            crate::tray::CloseDecision::Tray => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.minimize_to_tray(ctx);
            }
            crate::tray::CloseDecision::Quit => {
                // Real quit (tray 退出 / remembered quit / no tray).
                self.shutdown_backends();
            }
        }
    }

    fn minimize_to_tray(&self, ctx: &egui::Context) {
        // Minimize instead of Visible(false): eframe 0.33 on Windows stops
        // painting when the viewport is invisible, so tray Show never runs.
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    fn choose_close(&mut self, ctx: &egui::Context, action: CloseAction, dont_ask: bool) {
        self.close_dialog = None;
        if dont_ask {
            self.close_ask = false;
            self.close_action = action;
            let _ = self.commit_settings();
        }
        match action {
            CloseAction::Tray => self.minimize_to_tray(ctx),
            CloseAction::Quit => self.request_quit(ctx),
        }
    }

    fn show_close_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.close_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut choice = None;
        egui::Window::new("关闭主窗口")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("要退出程序，还是最小化到系统托盘继续下载？");
                ui.add_space(8.0);
                ui.checkbox(&mut state.dont_ask, "不再询问");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("最小化到托盘").clicked() {
                        choice = Some(CloseAction::Tray);
                    }
                    let quit = ui.add(
                        egui::Button::new(
                            egui::RichText::new("退出程序").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(185, 28, 28)),
                    );
                    if quit.clicked() {
                        choice = Some(CloseAction::Quit);
                    }
                });
            });
        if let Some(action) = choice {
            self.choose_close(ctx, action, state.dont_ask);
            return;
        }
        if open {
            self.close_dialog = Some(state);
        }
    }

    fn sync_file_name_from_url(&mut self) {
        if let Some((name, auto)) =
            next_auto_file_name(&self.url, &self.file_name, &self.auto_file_name)
        {
            self.file_name = name;
            self.auto_file_name = auto;
        }
    }

    fn submit_add(&mut self) {
        let url = self.url.trim().to_string();
        self.sync_file_name_from_url();
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
                .set_notice(Some("下载引擎已停止".into()));
            return;
        }
        self.url.clear();
        self.file_name.clear();
        self.auto_file_name.clear();
    }

    fn browse_save_dir(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("选择保存目录");
        let current = self.save_dir.trim();
        if !current.is_empty() {
            dialog = dialog.set_directory(current);
        }
        if let Some(path) = dialog.pick_folder() {
            self.save_dir = path.to_string_lossy().into_owned();
            self.commit_settings();
        }
    }

    /// Persists the settings fields. Returns false when a field is invalid.
    fn commit_settings(&mut self) -> bool {
        let save_dir = self.save_dir.trim().to_string();
        if save_dir.is_empty() {
            self.model
                .set_notice(Some("请选择保存目录".into()));
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
            autostart: true,
            close_ask: self.close_ask,
            close_action: self.close_action,
        }));
        true
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            self.model
                .set_notice(Some("下载引擎已停止".into()));
        }
    }
}

impl Drop for DownloadUi {
    fn drop(&mut self) {
        self.shutdown_backends();
    }
}

impl eframe::App for DownloadUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(400));
        self.apply_activation(ctx);
        self.apply_tray(ctx);
        self.handle_window_close(ctx);
        self.show_close_dialog(ctx);
        let (tasks, notice) = self.model.view();

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(banner) = &self.banner {
                ui.colored_label(egui::Color32::from_rgb(255, 186, 92), banner);
                ui.add_space(8.0);
            }

            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("保存目录");
                    ui.horizontal(|ui| {
                        let save = ui.add(
                            egui::TextEdit::singleline(&mut self.save_dir)
                                .desired_width(440.0)
                                .hint_text("选择或输入保存路径"),
                        );
                        if field_committed(&save, ui) {
                            self.commit_settings();
                        }
                        if ui.button("浏览…").clicked() {
                            self.browse_save_dir();
                        }
                    });
                    ui.end_row();

                    ui.label("下载地址");
                    let url_edit = ui.add(
                        egui::TextEdit::singleline(&mut self.url)
                            .desired_width(520.0)
                            .hint_text("http(s)://example.com/file.zip"),
                    );
                    if url_edit.changed() {
                        self.sync_file_name_from_url();
                    }
                    ui.end_row();

                    ui.label("文件名");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.file_name)
                                .desired_width(420.0)
                                .hint_text("file.zip"),
                        );
                        let start = ui.add(
                            egui::Button::new(
                                egui::RichText::new("开始下载").color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(37, 99, 235)),
                        );
                        if start.clicked() {
                            self.submit_add();
                        }
                        let clear = ui.add_enabled(!tasks.is_empty(), egui::Button::new("清空"));
                        if clear.clicked() {
                            self.send(Command::ClearAll);
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
            ui.add_space(4.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                if tasks.is_empty() {
                    ui.label("暂无下载任务。");
                    return;
                }
                let save_dir = self.save_dir.clone();
                let mut open_notice = None;
                task_table(ui, &tasks, &save_dir, |command| self.send(command), |msg| {
                    open_notice = Some(msg);
                });
                if let Some(msg) = open_notice {
                    self.model.set_notice(Some(msg));
                }
            });
        });
    }
}

fn task_table(
    ui: &mut egui::Ui,
    tasks: &[DownloadTask],
    save_dir: &str,
    mut send: impl FnMut(Command),
    mut on_notice: impl FnMut(String),
) {
    egui::Grid::new("tasks")
        .striped(true)
        .num_columns(6)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.strong("文件");
            ui.strong("状态");
            ui.strong("进度");
            ui.strong("速度");
            ui.strong("时间");
            ui.strong("操作");
            ui.end_row();

            for task in tasks {
                file_name_cell(ui, task, save_dir, &mut on_notice);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(status_name(task.status))
                            .color(status_color(task.status)),
                    );
                    if let Some(err) = &task.error {
                        ui.label(
                            egui::RichText::new(localize_engine_message(err))
                                .small()
                                .color(egui::Color32::from_rgb(220, 38, 38)),
                        );
                    }
                });
                progress_cell(ui, task);
                ui.label(speed_text(task));
                ui.monospace(format_task_created_at(task.created_at));
                ui.push_id(&task.id.0, |ui| {
                    ui.horizontal(|ui| {
                        let pause = task.status == TaskStatus::Active;
                        let resume = task.status == TaskStatus::Paused;
                        let cancel = can_cancel(task);
                        if ui.add_enabled(pause, egui::Button::new("暂停")).clicked() {
                            send(Command::Pause(task.id.clone()));
                        }
                        if ui
                            .add_enabled(resume, egui::Button::new("继续"))
                            .clicked()
                        {
                            send(Command::Resume(task.id.clone()));
                        }
                        if ui
                            .add_enabled(cancel, egui::Button::new("取消"))
                            .clicked()
                        {
                            send(Command::Cancel(task.id.clone()));
                        }
                        let file_path = Path::new(save_dir).join(&task.file_name);
                        let can_copy = task.status == TaskStatus::Complete && file_path.is_file();
                        if ui
                            .add_enabled(can_copy, egui::Button::new("复制"))
                            .on_hover_text("复制已下载的文件")
                            .clicked()
                        {
                            match crate::platform::copy_file_to_clipboard(&file_path) {
                                Ok(()) => on_notice("已复制文件，可粘贴到文件夹".into()),
                                Err(err) => on_notice(format!("无法复制文件：{err}")),
                            }
                        }
                    });
                });
                ui.end_row();
            }
        });
}

fn format_task_created_at(secs: u64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(secs as i64, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "—".into())
}

fn file_name_cell(
    ui: &mut egui::Ui,
    task: &DownloadTask,
    save_dir: &str,
    on_notice: &mut impl FnMut(String),
) {
    if task.status == TaskStatus::Complete {
        let response = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(&task.file_name)
                        .color(egui::Color32::from_rgb(37, 99, 235)),
                )
                .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("打开所在目录");
        if response.clicked() {
            let path = Path::new(save_dir).join(&task.file_name);
            if let Err(err) = crate::platform::reveal_in_folder(&path) {
                on_notice(format!("无法打开目录：{err}"));
            }
        }
        return;
    }
    ui.label(&task.file_name).on_hover_text(&task.url);
}

fn progress_cell(ui: &mut egui::Ui, task: &DownloadTask) {
    // Declared `size` (from the site/SDK) can disagree with aria2's actual
    // bytes. Once aria2 reports complete, always show 100%.
    if task.status == TaskStatus::Complete {
        ui.add(
            egui::ProgressBar::new(1.0)
                .desired_width(180.0)
                .show_percentage(),
        );
        return;
    }
    if let Some(total) = task.size.filter(|total| *total > 0) {
        let fraction = (task.completed_length as f32 / total as f32).clamp(0.0, 1.0);
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_width(180.0)
                .show_percentage(),
        );
        return;
    }
    if task.completed_length > 0 {
        ui.label(format_bytes(task.completed_length));
    } else {
        ui.label("—");
    }
}

fn status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Queued => "排队中",
        TaskStatus::Active => "下载中",
        TaskStatus::Paused => "已暂停",
        TaskStatus::Complete => "已完成",
        TaskStatus::Error => "失败",
    }
}

fn status_color(status: TaskStatus) -> egui::Color32 {
    match status {
        TaskStatus::Queued => egui::Color32::from_rgb(107, 114, 128),
        TaskStatus::Active => egui::Color32::from_rgb(37, 99, 235),
        TaskStatus::Paused => egui::Color32::from_rgb(217, 119, 6),
        TaskStatus::Complete => egui::Color32::from_rgb(22, 163, 74),
        TaskStatus::Error => egui::Color32::from_rgb(220, 38, 38),
    }
}

fn localize_engine_message(message: &str) -> String {
    match message {
        "cancelled" => "已取消".into(),
        "url required" => "请填写下载地址".into(),
        "only http:// and https:// URLs are allowed" => "仅支持 http:// 或 https:// 地址".into(),
        "only http:// URLs are allowed" => "仅支持 http:// 或 https:// 地址".into(),
        "url must not include user info" => "地址不能包含用户名和密码".into(),
        "task not found" => "任务不存在".into(),
        "task has no gid" => "任务尚未开始".into(),
        other => other.to_string(),
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
        return Err("限速必须是整数（KB/s）".into());
    }
    let kbps: u64 = text
        .parse()
        .map_err(|_| "限速必须是整数（KB/s）".to_string())?;
    if kbps == 0 {
        return Ok(None);
    }
    kbps.checked_mul(1024)
        .map(Some)
        .ok_or_else(|| "限速数值过大".into())
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
        Err("并行数须为 1–16".into())
    }
}

pub fn can_cancel(task: &DownloadTask) -> bool {
    // Finished rows are removable too (aria2 uses removeDownloadResult).
    match task.status {
        TaskStatus::Queued | TaskStatus::Error | TaskStatus::Complete => true,
        TaskStatus::Active | TaskStatus::Paused => task.gid.is_some(),
    }
}

pub fn validate_task_input(url: &str, file_name: &str) -> Result<(), String> {
    require_http_url(url).map_err(|err| localize_engine_message(&err))?;
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return Err("请填写文件名".into());
    }
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains('\0') {
        return Err("文件名不能包含路径".into());
    }
    Ok(())
}

/// When the file-name field is empty or still equal to the last auto value,
/// returns `(new_file_name, new_auto_file_name)` derived from `url`.
fn next_auto_file_name(
    url: &str,
    file_name: &str,
    auto_file_name: &str,
) -> Option<(String, String)> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let suggested = suggested_file_name(url);
    let current = file_name.trim();
    if current.is_empty() || current == auto_file_name {
        Some((suggested.clone(), suggested))
    } else {
        None
    }
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
            ui.heading("下载器无法启动");
            ui.add_space(12.0);
            ui.label(&self.message);
            ui.add_space(16.0);
            if ui.button("退出").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

pub fn run(mut app: DownloadUi) -> eframe::Result<()> {
    eframe::run_native(
        "下载器",
        native_options("下载器", [980.0, 680.0]),
        Box::new(move |cc| {
            crate::fonts::configure_fonts(&cc.egui_ctx);
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
    ctx.request_repaint();
}

pub fn run_error(
    message: &str,
    instance: crate::single_instance::InstanceLock,
) -> eframe::Result<()> {
    let message = message.to_string();
    eframe::run_native(
        "下载器",
        native_options("下载器", [560.0, 240.0]),
        Box::new(move |cc| {
            crate::fonts::configure_fonts(&cc.egui_ctx);
            Ok(Box::new(ErrorScreen::new(message, instance)))
        }),
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
        can_cancel, format_bytes, format_speed_limit_kbps, format_task_created_at,
        next_auto_file_name, parse_max_connections, parse_speed_limit_kbps, validate_task_input,
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
            "请填写下载地址"
        );
        assert_eq!(
            validate_task_input("http://example.com/a", "").unwrap_err(),
            "请填写文件名"
        );
        assert!(validate_task_input("http://example.com/a", "../secret").is_err());
        assert!(validate_task_input("http://example.com/a", "a/b.zip").is_err());
        assert!(validate_task_input("http://example.com/a", "a.zip").is_ok());
    }

    #[test]
    fn formats_created_at_as_local_datetime() {
        let text = format_task_created_at(1_700_000_000);
        assert_eq!(text.len(), "yyyy-mm-dd hh:mm:ss".len());
        assert!(text.chars().nth(4) == Some('-'));
        assert!(text.chars().nth(7) == Some('-'));
        assert!(text.chars().nth(10) == Some(' '));
        assert!(text.chars().nth(13) == Some(':'));
        assert!(text.chars().nth(16) == Some(':'));
    }

    #[test]
    fn fills_file_name_from_url_until_user_edits_it() {
        let (name, auto) =
            next_auto_file_name("https://host/path/aria2-1.37.0.zip", "", "").unwrap();
        assert_eq!(name, "aria2-1.37.0.zip");
        assert_eq!(auto, "aria2-1.37.0.zip");

        let (name, _) = next_auto_file_name(
            "https://host/other/b.bin",
            "aria2-1.37.0.zip",
            "aria2-1.37.0.zip",
        )
        .unwrap();
        assert_eq!(name, "b.bin");

        assert!(next_auto_file_name(
            "https://host/other/b.bin",
            "custom.bin",
            "aria2-1.37.0.zip",
        )
        .is_none());
    }

    #[test]
    fn rejects_non_http_urls_and_accepts_http_https() {
        assert_eq!(
            validate_task_input("ftp://example.com/a", "a.zip").unwrap_err(),
            "仅支持 http:// 或 https:// 地址"
        );
        assert!(validate_task_input("http://example.com/a", "a.zip").is_ok());
        assert!(validate_task_input("HTTP://example.com/a", "a.zip").is_ok());
        assert!(validate_task_input("https://example.com/a", "a.zip").is_ok());
        assert!(validate_task_input("HTTPS://example.com/a", "a.zip").is_ok());
        assert_eq!(
            validate_task_input("http://user:pass@example.com/a", "a.zip").unwrap_err(),
            "地址不能包含用户名和密码"
        );
        assert_eq!(
            validate_task_input("https://user:pass@example.com/a", "a.zip").unwrap_err(),
            "地址不能包含用户名和密码"
        );
    }

    #[test]
    fn cancel_enabled_for_queued_and_error_without_gid() {
        assert!(can_cancel(&task(TaskStatus::Queued, None)));
        assert!(can_cancel(&task(TaskStatus::Error, None)));
        assert!(can_cancel(&task(TaskStatus::Complete, None)));
        assert!(can_cancel(&task(TaskStatus::Complete, Some("gid-1"))));
        assert!(can_cancel(&task(TaskStatus::Error, Some("gid-1"))));
        assert!(can_cancel(&task(TaskStatus::Active, Some("gid-1"))));
        assert!(can_cancel(&task(TaskStatus::Paused, Some("gid-1"))));
        assert!(!can_cancel(&task(TaskStatus::Active, None)));
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
