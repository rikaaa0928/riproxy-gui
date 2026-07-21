use crate::backend::{
    BackendEvent, BackendKind, BackendLaunch, ProxyBackend, available_kinds, create_backend,
};
use crate::config::{self, AppConfig, Profile};
use crate::observe::{ObserveState, format_bps, format_bytes, format_uptime};
use crate::platform::{self, TrayAction, TrayHandle};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, Eq, PartialEq)]
enum View {
    Overview,
    Connections,
    Routes,
    Profiles,
    Logs,
    Settings,
}

pub struct ProxyGuiApp {
    config: AppConfig,
    backend: Option<Box<dyn ProxyBackend>>,
    backend_kind: Option<BackendKind>,
    observe: ObserveState,
    config_editor: ConfigEditor,
    new_profile_name: String,
    active_profile_name: Option<String>,
    pending_restart_profile_name: Option<String>,
    tray: Option<TrayHandle>,
    tray_error_reported: bool,
    window_visible: bool,
    quit_requested: bool,
    view: View,
    logs: VecDeque<String>,
    last_error: Option<String>,
}

#[derive(Default)]
struct ConfigEditor {
    profile_index: Option<usize>,
    path: String,
    text: String,
    dirty: bool,
    status: Option<String>,
}

impl ProxyGuiApp {
    pub fn new(cc: &eframe::CreationContext<'_>, start_minimized: bool) -> Self {
        setup_style(&cc.egui_ctx);
        let mut config = config::load();
        if start_minimized {
            config.run_in_tray = true;
        }
        let active_profile_name = config.active_profile.clone();
        let mut app = Self {
            config,
            backend: None,
            backend_kind: None,
            observe: ObserveState::new(),
            config_editor: ConfigEditor::default(),
            new_profile_name: String::new(),
            active_profile_name,
            pending_restart_profile_name: None,
            tray: None,
            tray_error_reported: false,
            window_visible: !start_minimized,
            quit_requested: false,
            view: View::Overview,
            logs: VecDeque::new(),
            last_error: None,
        };
        app.restore_active_profile();
        app
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.config.profiles.get(self.config.selected_profile)
    }

    fn profile_index_by_name(&self, name: &str) -> Option<usize> {
        let name = config::normalize_profile_name(name);
        self.config
            .profiles
            .iter()
            .position(|profile| profile.name == name)
    }

    fn normalize_profile_paths(&mut self) {
        config::normalize_profiles(&mut self.config);
    }

    fn normalize_selected_profile_path(&mut self) {
        config::normalize_profiles(&mut self.config);
    }

    fn is_running(&self) -> bool {
        self.backend
            .as_ref()
            .is_some_and(|backend| backend.is_running())
    }

    fn active_backend_kind(&self) -> Option<BackendKind> {
        self.backend.as_ref().map(|backend| backend.kind())
    }

    fn ensure_backend(&mut self, kind: BackendKind) -> anyhow::Result<()> {
        if self.backend_kind == Some(kind) && self.backend.is_some() {
            return Ok(());
        }
        if self.is_running() {
            anyhow::bail!("stop the active backend before switching backend type");
        }
        self.backend = Some(create_backend(kind)?);
        self.backend_kind = Some(kind);
        Ok(())
    }

    fn restore_active_profile(&mut self) {
        self.normalize_profile_paths();
        let Some(profile_name) = self.config.active_profile.clone() else {
            return;
        };
        if !self.start_profile_by_name(&profile_name) {
            self.clear_active_profile();
        }
    }

    fn start_profile_by_name(&mut self, profile_name: &str) -> bool {
        let Some(index) = self.profile_index_by_name(profile_name) else {
            self.push_error(format!("profile not found: {profile_name}"));
            return false;
        };
        self.config.selected_profile = index;
        self.start_selected_profile()
    }

    fn start_selected_profile(&mut self) -> bool {
        self.normalize_selected_profile_path();
        let Some(profile) = self.selected_profile().cloned() else {
            self.push_error("no profile selected");
            return false;
        };
        if self.config_editor.profile_index == Some(self.config.selected_profile)
            && self.config_editor.path == profile.config_path
            && self.config_editor.dirty
        {
            self.push_error("save the edited config file before starting");
            return false;
        }
        if !profile.backend.is_available() {
            self.push_error(format!("{} backend is not enabled", profile.backend));
            return false;
        }
        if let Err(e) = self.ensure_backend(profile.backend) {
            self.push_error(e.to_string());
            return false;
        }
        if let Err(e) = ensure_profile_config_file(&profile) {
            self.push_error(format!("create default config failed: {e}"));
            return false;
        }

        let launch = BackendLaunch {
            config_path: PathBuf::from(profile.config_path),
            auto_reload: profile.auto_reload,
        };
        let result = self
            .backend
            .as_mut()
            .expect("backend should be initialized")
            .start(launch);
        match result {
            Ok(()) => {
                self.set_active_profile(Some(profile.name));
                self.push_log(format!("{} start requested", profile.backend));
                true
            }
            Err(e) => {
                self.push_error(e.to_string());
                false
            }
        }
    }

    fn stop_backend(&mut self) {
        self.pending_restart_profile_name = None;
        if let Some(backend) = self.backend.as_mut() {
            match backend.stop() {
                Ok(()) => {
                    self.clear_active_profile();
                    self.push_log("stop requested");
                }
                Err(e) => self.push_error(e.to_string()),
            }
        }
    }

    fn restart_active_profile(&mut self) {
        let Some(profile_name) = self.active_profile_name.clone() else {
            self.push_error("no active profile to restart");
            return;
        };

        if !self.is_running() {
            if !self.start_profile_by_name(&profile_name) {
                self.clear_active_profile();
            }
            return;
        }

        if let Some(backend) = self.backend.as_mut() {
            match backend.stop() {
                Ok(()) => {
                    self.pending_restart_profile_name = Some(profile_name);
                    self.push_log("restart requested");
                }
                Err(e) => self.push_error(e.to_string()),
            }
        }
    }

    fn reload_backend(&mut self) {
        if let Some(backend) = self.backend.as_mut() {
            match backend.reload() {
                Ok(()) => self.push_log("reload requested"),
                Err(e) => self.push_error(e.to_string()),
            }
        }
    }

    fn poll_backend(&mut self) {
        let registry = self
            .backend
            .as_ref()
            .and_then(|backend| backend.observe_registry());
        self.observe.update(registry);

        if let Some(backend) = self.backend.as_mut() {
            for event in backend.drain_events() {
                match event {
                    BackendEvent::Info(message) => self.push_log(message),
                    BackendEvent::Error(message) => self.push_error(message),
                    BackendEvent::Stopped { error } => {
                        if let Some(error) = error {
                            self.push_error(format!("backend stopped: {error}"));
                        } else {
                            self.push_log("backend stopped");
                        }
                        if let Some(profile_name) = self.pending_restart_profile_name.take() {
                            self.push_log(format!("restarting profile {profile_name}"));
                            if !self.start_profile_by_name(&profile_name) {
                                self.clear_active_profile();
                            }
                        } else {
                            self.clear_active_profile();
                        }
                    }
                }
            }
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        self.logs.push_front(message.into());
        while self.logs.len() > 200 {
            self.logs.pop_back();
        }
    }

    fn push_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.last_error = Some(message.clone());
        self.push_log(format!("error: {message}"));
    }

    fn save_config(&mut self) {
        self.normalize_profile_paths();
        match config::save(&self.config) {
            Ok(()) => self.push_log("profiles saved"),
            Err(e) => self.push_error(e.to_string()),
        }
    }

    fn save_config_silent(&mut self) {
        self.normalize_profile_paths();
        if let Err(e) = config::save(&self.config) {
            self.push_error(e.to_string());
        }
    }

    fn set_active_profile(&mut self, profile_name: Option<String>) {
        self.active_profile_name = profile_name.clone();
        self.config.active_profile = profile_name;
        self.save_config_silent();
    }

    fn clear_active_profile(&mut self) {
        if self.active_profile_name.is_some() || self.config.active_profile.is_some() {
            self.set_active_profile(None);
        }
    }

    fn ensure_tray(&mut self, ctx: &egui::Context) {
        if !self.config.run_in_tray {
            self.tray = None;
            return;
        }
        platform::install_event_handlers(ctx.clone());
        if self.tray.is_some() {
            return;
        }

        match TrayHandle::new() {
            Ok(tray) => {
                self.tray = Some(tray);
                self.tray_error_reported = false;
                self.push_log("tray enabled");
            }
            Err(e) => {
                if !self.tray_error_reported {
                    self.push_error(format!("enable tray failed: {e}"));
                    self.tray_error_reported = true;
                }
                if !self.window_visible {
                    self.show_window(ctx);
                }
            }
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        for action in platform::poll_tray_actions() {
            match action {
                TrayAction::Show => self.show_window(ctx),
                TrayAction::Hide => self.hide_window(ctx),
                TrayAction::Quit => self.quit(ctx),
            }
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && self.config.run_in_tray && !self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.window_visible = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        if self.config.run_in_tray {
            self.window_visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quit_requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn set_launch_at_login(&mut self, enabled: bool) {
        if enabled {
            self.config.run_in_tray = true;
        }
        match platform::set_launch_at_login(enabled) {
            Ok(()) => {
                self.config.launch_at_login = enabled;
                self.save_config();
                self.push_log(if enabled {
                    "launch at login enabled"
                } else {
                    "launch at login disabled"
                });
            }
            Err(e) => self.push_error(e.to_string()),
        }
    }

    fn add_profile(&mut self) {
        let raw_name = self.new_profile_name.trim();
        if raw_name.is_empty() {
            self.push_error("profile name is required");
            return;
        }

        let name = config::normalize_profile_name(raw_name);
        if self
            .config
            .profiles
            .iter()
            .any(|profile| config::normalize_profile_name(&profile.name) == name)
        {
            self.push_error(format!("profile already exists: {name}"));
            return;
        }

        let backend = available_kinds()
            .into_iter()
            .next()
            .unwrap_or(BackendKind::Leaf);
        self.config.profiles.push(Profile {
            config_path: config::profile_config_path(&name, backend),
            name,
            backend,
            auto_reload: true,
        });
        self.config.selected_profile = self.config.profiles.len() - 1;
        self.new_profile_name.clear();
        self.normalize_profile_paths();
    }

    fn load_profile_config_file(&mut self) {
        self.normalize_profile_paths();
        let profile_index = self.config.selected_profile;
        let Some(profile) = self.config.profiles.get(profile_index) else {
            self.push_error("no profile selected");
            return;
        };
        let backend = profile.backend;
        let path = profile.config_path.clone();

        match fs::read_to_string(&path) {
            Ok(text) => {
                self.config_editor = ConfigEditor {
                    profile_index: Some(profile_index),
                    path: path.clone(),
                    text,
                    dirty: false,
                    status: Some(format!("loaded {path}")),
                };
                self.push_log(format!("loaded config file {path}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let text = default_proxy_config_template(backend);
                if let Err(e) = write_proxy_config_file(&path, &text) {
                    self.push_error(format!("create config file failed: {e}"));
                    return;
                }
                self.config_editor = ConfigEditor {
                    profile_index: Some(profile_index),
                    path: path.clone(),
                    text,
                    dirty: false,
                    status: Some(format!("created {path}")),
                };
                self.push_log(format!("created config file {path}"));
            }
            Err(e) => self.push_error(format!("load config file failed: {e}")),
        }
    }

    fn save_profile_config_file(&mut self) {
        self.normalize_profile_paths();
        let Some(profile) = self.selected_profile() else {
            self.push_error("no profile selected");
            return;
        };
        let path = profile.config_path.clone();
        self.config_editor.path = path.clone();
        match write_proxy_config_file(&path, &self.config_editor.text) {
            Ok(()) => {
                self.config_editor.dirty = false;
                self.config_editor.status = Some(format!("saved {path}"));
                self.push_log(format!("saved config file {path}"));
            }
            Err(e) => self.push_error(format!("save config file failed: {e}")),
        }
    }

    fn validate_profile_config_file(&mut self) {
        self.normalize_selected_profile_path();
        let Some(profile) = self.selected_profile().cloned() else {
            self.push_error("no profile selected");
            return;
        };
        let path = PathBuf::from(&profile.config_path);
        let result = match profile.backend {
            BackendKind::Leaf => leaf_validate_config(&path),
            BackendKind::Rog => rog_validate_config(&path),
        };
        match result {
            Ok(()) => {
                self.config_editor.status = Some("config is valid".to_string());
                self.push_log(format!("{} config is valid", profile.backend));
            }
            Err(e) => self.push_error(format!("config validation failed: {e}")),
        }
    }

    fn draw_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("RiProxy");
            ui.separator();

            let selected_name = self
                .selected_profile()
                .map(|profile| profile.name.as_str())
                .unwrap_or("No profile");
            egui::ComboBox::from_id_salt("profile-select")
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (index, profile) in self.config.profiles.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.config.selected_profile,
                            index,
                            profile.name.as_str(),
                        );
                    }
                });

            let running = self.is_running();
            if ui
                .add_enabled(
                    !running,
                    egui::Button::new("Start").min_size(Vec2::new(76.0, 28.0)),
                )
                .clicked()
            {
                self.start_selected_profile();
            }
            if ui
                .add_enabled(
                    running,
                    egui::Button::new("Stop").min_size(Vec2::new(76.0, 28.0)),
                )
                .clicked()
            {
                self.stop_backend();
            }
            if ui
                .add_enabled(
                    running,
                    egui::Button::new(if self.active_backend_kind() == Some(BackendKind::Rog) {
                        "Restart"
                    } else {
                        "Reload"
                    })
                    .min_size(Vec2::new(76.0, 28.0)),
                )
                .clicked()
            {
                if self.active_backend_kind() == Some(BackendKind::Rog) {
                    self.restart_active_profile();
                } else {
                    self.reload_backend();
                }
            }

            ui.separator();
            let status = if running { "Running" } else { "Stopped" };
            let color = if running {
                Color32::from_rgb(74, 178, 112)
            } else {
                Color32::from_rgb(150, 155, 165)
            };
            ui.colored_label(color, status);

            if let Some(error) = &self.last_error {
                ui.separator();
                ui.colored_label(Color32::from_rgb(222, 96, 96), error);
            }
        });
    }

    fn draw_nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        nav_button(ui, &mut self.view, View::Overview, "Overview");
        nav_button(ui, &mut self.view, View::Connections, "Connections");
        nav_button(ui, &mut self.view, View::Routes, "Routes");
        nav_button(ui, &mut self.view, View::Profiles, "Profiles");
        nav_button(ui, &mut self.view, View::Logs, "Logs");
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            nav_button(ui, &mut self.view, View::Settings, "Settings");
        });
    }

    fn draw_overview(&mut self, ui: &mut egui::Ui) {
        if let Some(overview) = &self.observe.overview {
            metrics_row(
                ui,
                [
                    ("Down", format_bps(overview.rx_bps)),
                    ("Up", format_bps(overview.tx_bps)),
                    ("Active", overview.active_connections.to_string()),
                    ("Uptime", format_uptime(overview.uptime_seconds)),
                ],
            );
            ui.add_space(10.0);
            metrics_row(
                ui,
                [
                    ("Downloaded", format_bytes(overview.total_rx_bytes)),
                    ("Uploaded", format_bytes(overview.total_tx_bytes)),
                    ("Sites", overview.sites.to_string()),
                    ("Routes", overview.routes.to_string()),
                ],
            );
            ui.add_space(18.0);
            self.draw_bandwidth_chart(ui);
        } else {
            ui.label("No active backend");
        }

        ui.add_space(18.0);
        ui.heading("Top Sites");
        self.draw_sites_table(ui, 8, "overview-top-sites-table", Some(260.0));
    }

    fn draw_bandwidth_chart(&self, ui: &mut egui::Ui) {
        let desired = Vec2::new(ui.available_width(), 180.0);
        let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );

        let max = self
            .observe
            .history
            .iter()
            .map(|point| point.rx_bps.max(point.tx_bps))
            .max()
            .unwrap_or(1)
            .max(1) as f32;

        let to_pos = |index: usize, value: u64| {
            let len = self.observe.history.len().saturating_sub(1).max(1) as f32;
            let x = rect.left() + rect.width() * (index as f32 / len);
            let y = rect.bottom() - rect.height() * ((value as f32 / max).clamp(0.0, 1.0));
            egui::pos2(x, y)
        };

        let rx: Vec<_> = self
            .observe
            .history
            .iter()
            .enumerate()
            .map(|(index, point)| to_pos(index, point.rx_bps))
            .collect();
        let tx: Vec<_> = self
            .observe
            .history
            .iter()
            .enumerate()
            .map(|(index, point)| to_pos(index, point.tx_bps))
            .collect();
        painter.add(egui::Shape::line(
            rx,
            Stroke::new(2.0, Color32::from_rgb(82, 152, 221)),
        ));
        painter.add(egui::Shape::line(
            tx,
            Stroke::new(2.0, Color32::from_rgb(220, 161, 74)),
        ));
        painter.text(
            rect.left_top() + Vec2::new(10.0, 8.0),
            egui::Align2::LEFT_TOP,
            format!("Peak {}", format_bps(max as u64)),
            egui::FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );
    }

    fn draw_connections(&self, ui: &mut egui::Ui) {
        TableBuilder::new(ui)
            .id_salt("connections-table")
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .auto_shrink([false, false])
            .column(Column::exact(64.0))
            .column(Column::initial(180.0).at_least(160.0).clip(true))
            .column(Column::remainder().at_least(240.0).clip(true))
            .column(Column::initial(160.0).at_least(140.0).clip(true))
            .column(Column::exact(96.0))
            .column(Column::exact(96.0))
            .header(24.0, |mut header| {
                header.col(|ui| table_header(ui, "Net"));
                header.col(|ui| table_header(ui, "Source"));
                header.col(|ui| table_header(ui, "Destination"));
                header.col(|ui| table_header(ui, "Route"));
                header.col(|ui| table_header(ui, "Down"));
                header.col(|ui| table_header(ui, "Up"));
            })
            .body(|mut body| {
                for conn in &self.observe.connections {
                    body.row(24.0, |mut row| {
                        row.col(|ui| {
                            ui.label(conn.network.as_str());
                        });
                        row.col(|ui| {
                            ui.label(conn.source.as_str());
                        });
                        row.col(|ui| {
                            ui.label(conn.destination.as_str());
                        });
                        row.col(|ui| {
                            ui.label(conn.route.as_deref().unwrap_or("-"));
                        });
                        row.col(|ui| {
                            ui.label(format_bytes(conn.rx_bytes));
                        });
                        row.col(|ui| {
                            ui.label(format_bytes(conn.tx_bytes));
                        });
                    });
                }
            });
    }

    fn draw_routes(&self, ui: &mut egui::Ui) {
        ui.heading("Routes");
        let table_height = ((ui.available_height() - 78.0) * 0.48).clamp(180.0, 360.0);
        TableBuilder::new(ui)
            .id_salt("routes-table")
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .auto_shrink([false, false])
            .max_scroll_height(table_height)
            .column(Column::remainder().at_least(220.0).clip(true))
            .column(Column::initial(180.0).at_least(160.0).clip(true))
            .column(Column::exact(72.0))
            .column(Column::exact(96.0))
            .column(Column::exact(96.0))
            .header(24.0, |mut header| {
                header.col(|ui| table_header(ui, "Route"));
                header.col(|ui| table_header(ui, "Outbound"));
                header.col(|ui| table_header(ui, "Active"));
                header.col(|ui| table_header(ui, "Down"));
                header.col(|ui| table_header(ui, "Up"));
            })
            .body(|mut body| {
                for route in &self.observe.routes {
                    body.row(24.0, |mut row| {
                        row.col(|ui| {
                            ui.label(route.route.as_str());
                        });
                        row.col(|ui| {
                            ui.label(route.outbound.as_deref().unwrap_or("-"));
                        });
                        row.col(|ui| {
                            ui.label(route.active_connections.to_string());
                        });
                        row.col(|ui| {
                            ui.label(format_bps(route.rx_bps));
                        });
                        row.col(|ui| {
                            ui.label(format_bps(route.tx_bps));
                        });
                    });
                }
            });

        ui.add_space(18.0);
        ui.heading("Sites");
        self.draw_sites_table(ui, usize::MAX, "routes-sites-table", None);
    }

    fn draw_sites_table(
        &self,
        ui: &mut egui::Ui,
        limit: usize,
        id_salt: &'static str,
        max_scroll_height: Option<f32>,
    ) {
        let mut table = TableBuilder::new(ui)
            .id_salt(id_salt)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .auto_shrink([false, false])
            .column(Column::remainder().at_least(260.0).clip(true))
            .column(Column::exact(72.0))
            .column(Column::exact(96.0))
            .column(Column::exact(96.0))
            .column(Column::exact(110.0));
        if let Some(max_scroll_height) = max_scroll_height {
            table = table.max_scroll_height(max_scroll_height);
        }

        table
            .header(24.0, |mut header| {
                header.col(|ui| table_header(ui, "Site"));
                header.col(|ui| table_header(ui, "Active"));
                header.col(|ui| table_header(ui, "Down"));
                header.col(|ui| table_header(ui, "Up"));
                header.col(|ui| table_header(ui, "Total"));
            })
            .body(|mut body| {
                for site in self.observe.sites.iter().take(limit) {
                    body.row(24.0, |mut row| {
                        row.col(|ui| {
                            ui.label(site.site.as_str());
                        });
                        row.col(|ui| {
                            ui.label(site.active_connections.to_string());
                        });
                        row.col(|ui| {
                            ui.label(format_bps(site.rx_bps));
                        });
                        row.col(|ui| {
                            ui.label(format_bps(site.tx_bps));
                        });
                        row.col(|ui| {
                            ui.label(format_bytes(site.total_rx_bytes + site.total_tx_bytes));
                        });
                    });
                }
            });
    }

    fn draw_profiles(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("New");
            let name_response = ui.add(
                egui::TextEdit::singleline(&mut self.new_profile_name)
                    .desired_width(180.0)
                    .hint_text("profile name"),
            );
            if name_response.changed() {
                self.new_profile_name = self.new_profile_name.to_lowercase();
            }
            let add_requested = ui.button("Add").clicked()
                || (name_response.lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if add_requested {
                self.add_profile();
            }
            if ui
                .add_enabled(self.config.profiles.len() > 1, egui::Button::new("Remove"))
                .clicked()
            {
                self.config.profiles.remove(self.config.selected_profile);
                self.config.selected_profile = self
                    .config
                    .selected_profile
                    .min(self.config.profiles.len().saturating_sub(1));
                self.normalize_profile_paths();
            }
            if ui.button("Save").clicked() {
                self.save_config();
            }
        });
        ui.separator();

        self.normalize_profile_paths();
        let mut selected_profile = self.config.selected_profile;

        egui::Grid::new("profile-editor")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .show(ui, |ui| {
                ui.label("Name");
                let selected_name = self
                    .config
                    .profiles
                    .get(selected_profile)
                    .map(|profile| profile.name.as_str())
                    .unwrap_or("No profile");
                egui::ComboBox::from_id_salt("profiles-page-profile-select")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for (index, profile) in self.config.profiles.iter().enumerate() {
                            ui.selectable_value(
                                &mut selected_profile,
                                index,
                                profile.name.as_str(),
                            );
                        }
                    });
                ui.end_row();
            });

        self.config.selected_profile = selected_profile;
        let selected_profile = self.config.selected_profile;
        let Some(profile) = self.config.profiles.get(selected_profile) else {
            ui.label("No profile selected");
            return;
        };
        let mut backend = profile.backend;
        let mut auto_reload = profile.auto_reload;

        egui::Grid::new("profile-settings")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .show(ui, |ui| {
                ui.label("Backend");
                egui::ComboBox::from_id_salt("backend-kind")
                    .selected_text(backend.label())
                    .show_ui(ui, |ui| {
                        for kind in available_kinds() {
                            ui.selectable_value(&mut backend, kind, kind.label());
                        }
                    });
                ui.end_row();

                ui.label("Config");
                ui.monospace(config::profile_config_path(profile.name.as_str(), backend));
                ui.end_row();

                ui.label("Auto reload");
                ui.checkbox(&mut auto_reload, "");
                ui.end_row();
            });

        if let Some(profile) = self.config.profiles.get_mut(selected_profile) {
            profile.backend = backend;
            profile.auto_reload = auto_reload;
        }
        self.normalize_profile_paths();

        ui.add_space(18.0);
        self.draw_config_editor(ui);
    }

    fn draw_config_editor(&mut self, ui: &mut egui::Ui) {
        let Some(profile) = self.selected_profile().cloned() else {
            return;
        };
        let loaded_current = self.config_editor.profile_index == Some(self.config.selected_profile)
            && self.config_editor.path == profile.config_path;

        ui.heading("Config File");
        ui.horizontal(|ui| {
            ui.label("Profile path");
            ui.monospace(profile.config_path.as_str());
            if !loaded_current {
                ui.colored_label(Color32::from_rgb(220, 161, 74), "not loaded");
            } else if self.config_editor.dirty {
                ui.colored_label(Color32::from_rgb(220, 161, 74), "modified");
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Load").clicked() {
                self.load_profile_config_file();
            }
            if ui
                .add_enabled(loaded_current, egui::Button::new("Save File"))
                .clicked()
            {
                self.save_profile_config_file();
            }
            if ui.button("Validate").clicked() {
                self.validate_profile_config_file();
            }
            if let Some(status) = &self.config_editor.status {
                ui.label(status);
            }
        });

        if loaded_current {
            let response = egui::TextEdit::multiline(&mut self.config_editor.text)
                .code_editor()
                .desired_rows(30)
                .desired_width(f32::INFINITY)
                .show(ui)
                .response;
            if response.changed() {
                self.config_editor.dirty = true;
                self.config_editor.status = Some("modified".to_string());
            }
        } else {
            ui.add_space(6.0);
            ui.label("Load the current profile config before editing.");
        }
    }

    fn draw_logs(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for line in &self.logs {
                ui.monospace(line);
            }
        });
    }

    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runtime");
        egui::Grid::new("settings-runtime")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .show(ui, |ui| {
                ui.label("Enabled backends");
                ui.label(
                    available_kinds()
                        .into_iter()
                        .map(|kind| kind.label())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                ui.end_row();

                ui.label("Active backend");
                ui.label(
                    self.backend
                        .as_ref()
                        .map(|backend| backend.kind().label())
                        .unwrap_or("-"),
                );
                ui.end_row();

                ui.label("Active profile");
                ui.label(self.active_profile_name.as_deref().unwrap_or("-"));
                ui.end_row();

                ui.label("Observe source");
                ui.label(if self.observe.overview.is_some() {
                    "embedded"
                } else {
                    "-"
                });
                ui.end_row();
            });

        ui.add_space(18.0);
        ui.heading("System");
        egui::Grid::new("settings-system")
            .num_columns(2)
            .spacing([18.0, 10.0])
            .show(ui, |ui| {
                ui.label("Run in tray");
                let mut run_in_tray = self.config.run_in_tray;
                if ui.checkbox(&mut run_in_tray, "").changed() {
                    self.config.run_in_tray = run_in_tray;
                    if !run_in_tray && self.config.launch_at_login {
                        self.set_launch_at_login(false);
                    } else {
                        self.save_config();
                    }
                }
                ui.end_row();

                ui.label("Launch at login");
                let mut launch_at_login = self.config.launch_at_login;
                if ui.checkbox(&mut launch_at_login, "").changed() {
                    self.set_launch_at_login(launch_at_login);
                }
                ui.end_row();

                ui.label("Launch state");
                match platform::is_launch_at_login_enabled() {
                    Ok(enabled) => ui.label(if enabled { "enabled" } else { "disabled" }),
                    Err(e) => ui
                        .colored_label(Color32::from_rgb(222, 96, 96), format!("unavailable: {e}")),
                };
                ui.end_row();
            });
    }
}

impl eframe::App for ProxyGuiApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close_request(ctx);
        self.ensure_tray(ctx);
        self.poll_tray(ctx);
        self.poll_backend();
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            self.draw_top_bar(ui);
            ui.separator();

            let rect = ui.available_rect_before_wrap();
            let nav_width = 160.0;
            let gap = 18.0;
            let nav_rect = egui::Rect::from_min_max(
                rect.min,
                egui::pos2((rect.left() + nav_width).min(rect.right()), rect.bottom()),
            );
            let content_rect = egui::Rect::from_min_max(
                egui::pos2((nav_rect.right() + gap).min(rect.right()), rect.top()),
                rect.max,
            );

            ui.painter().vline(
                nav_rect.right() + gap * 0.5,
                rect.y_range(),
                Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            );

            let mut nav_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(nav_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            self.draw_nav(&mut nav_ui);

            let mut content_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(content_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            match self.view {
                View::Connections => self.draw_connections(&mut content_ui),
                View::Routes => self.draw_routes(&mut content_ui),
                _ => {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(&mut content_ui, |ui| match self.view {
                            View::Overview => self.draw_overview(ui),
                            View::Profiles => self.draw_profiles(ui),
                            View::Logs => self.draw_logs(ui),
                            View::Settings => self.draw_settings(ui),
                            View::Connections | View::Routes => unreachable!(),
                        });
                }
            }
        });
    }

    fn on_exit(&mut self) {
        self.config.active_profile = self
            .is_running()
            .then(|| self.active_profile_name.clone())
            .flatten();
        let _ = config::save(&self.config);
        if let Some(backend) = self.backend.as_mut() {
            let _ = backend.stop();
        }
    }
}

fn setup_style(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*ctx.style_of(theme)).clone();
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.visuals.widgets.inactive.corner_radius = 4.0.into();
        style.visuals.widgets.hovered.corner_radius = 4.0.into();
        style.visuals.widgets.active.corner_radius = 4.0.into();
        ctx.set_style_of(theme, style);
    }
}

fn nav_button(ui: &mut egui::Ui, current: &mut View, target: View, label: &str) {
    let selected = *current == target;
    if ui
        .add_sized([128.0, 32.0], egui::Button::selectable(selected, label))
        .clicked()
    {
        *current = target;
    }
}

fn metrics_row(ui: &mut egui::Ui, items: [(&str, String); 4]) {
    let gap = 24.0;
    let column_width = ((ui.available_width() - gap * 3.0) / 4.0).max(132.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (label, value) in items {
            ui.allocate_ui_with_layout(
                Vec2::new(column_width, 58.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| metric(ui, label, value),
            );
        }
    });
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    ui.vertical(|ui| {
        ui.add(
            egui::Label::new(RichText::new(label).color(ui.visuals().weak_text_color())).truncate(),
        );
        ui.add(egui::Label::new(RichText::new(value).size(22.0).strong()).truncate());
    });
}

fn table_header(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).strong());
}

fn default_proxy_config_template(kind: BackendKind) -> String {
    match kind {
        BackendKind::Leaf => r#"[General]
loglevel = info
dns-server = 1.1.1.1
socks-interface = 127.0.0.1
socks-port = 1080

[Proxy]
Direct = direct
"#
        .to_string(),
        BackendKind::Rog => r#"[[listener]]
name = "socks5_inbound"
endpoint = "127.0.0.1:1080"
proto = "socks5"
router = "main_router"

[[connector]]
name = "direct"
proto = "tcp"

[[router]]
name = "main_router"
default = "direct"
"#
        .to_string(),
    }
}

fn ensure_profile_config_file(profile: &Profile) -> std::io::Result<()> {
    let path = PathBuf::from(config::profile_config_path(&profile.name, profile.backend));
    if path.exists() {
        return Ok(());
    }
    write_proxy_config_file(path, &default_proxy_config_template(profile.backend))
}

fn write_proxy_config_file(path: impl Into<PathBuf>, contents: &str) -> std::io::Result<()> {
    let path = path.into();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

fn leaf_validate_config(path: &PathBuf) -> anyhow::Result<()> {
    #[cfg(feature = "backend-leaf")]
    {
        leaf::test_config(&path.to_string_lossy())?;
        Ok(())
    }
    #[cfg(not(feature = "backend-leaf"))]
    {
        let _ = path;
        anyhow::bail!("leaf backend is not enabled")
    }
}

fn rog_validate_config(path: &PathBuf) -> anyhow::Result<()> {
    #[cfg(feature = "backend-rog")]
    {
        rog::load_config_file(path)?;
        Ok(())
    }
    #[cfg(not(feature = "backend-rog"))]
    {
        let _ = path;
        anyhow::bail!("rog backend is not enabled")
    }
}
