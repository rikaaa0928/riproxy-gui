use anyhow::{Context, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

const APP_NAME: &str = "RiProxy";
const APP_ID: &str = "moe.rikaaa0928.riproxy-gui";
const AUTOSTART_ARG: &str = "--minimized";
const MENU_SHOW: &str = "riproxy-show";
const MENU_HIDE: &str = "riproxy-hide";
const MENU_QUIT: &str = "riproxy-quit";
const ICON_BYTES: &[u8] = include_bytes!("../assets/icon.png");
static TRAY_ACTIONS: Mutex<Vec<TrayAction>> = Mutex::new(Vec::new());
static EVENT_HANDLERS_INSTALLED: OnceLock<()> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction {
    Show,
    Hide,
    Quit,
}

pub struct TrayHandle {
    _icon: TrayIcon,
}

impl TrayHandle {
    pub fn new() -> Result<Self> {
        let menu = Menu::new();
        let show = MenuItem::with_id(MENU_SHOW, "Show", true, None);
        let hide = MenuItem::with_id(MENU_HIDE, "Hide", true, None);
        let quit = MenuItem::with_id(MENU_QUIT, "Quit", true, None);
        menu.append_items(&[&show, &hide, &PredefinedMenuItem::separator(), &quit])
            .context("create tray menu")?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(APP_NAME)
            .with_icon(tray_icon()?)
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(true)
            .build()
            .context("create tray icon")?;

        Ok(Self { _icon: icon })
    }
}

pub fn install_event_handlers(ctx: eframe::egui::Context) {
    EVENT_HANDLERS_INSTALLED.get_or_init(|| {
        let tray_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let Some(action) = tray_event_action(event) {
                push_tray_action(action);
                tray_ctx.request_repaint();
            }
        }));

        MenuEvent::set_event_handler(Some(move |event| {
            if let Some(action) = menu_event_action(&event) {
                push_tray_action(action);
                ctx.request_repaint();
            }
        }));
    });
}

pub fn poll_tray_actions() -> Vec<TrayAction> {
    let mut actions = TRAY_ACTIONS
        .lock()
        .map(|mut actions| actions.drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        if let Some(action) = menu_event_action(&event) {
            actions.push(action);
        }
    }
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if let Some(action) = tray_event_action(event) {
            actions.push(action);
        }
    }
    actions
}

fn push_tray_action(action: TrayAction) {
    if let Ok(mut actions) = TRAY_ACTIONS.lock() {
        actions.push(action);
    }
}

fn menu_event_action(event: &MenuEvent) -> Option<TrayAction> {
    if event.id == MENU_SHOW {
        Some(TrayAction::Show)
    } else if event.id == MENU_HIDE {
        Some(TrayAction::Hide)
    } else if event.id == MENU_QUIT {
        Some(TrayAction::Quit)
    } else {
        None
    }
}

fn tray_event_action(event: TrayIconEvent) -> Option<TrayAction> {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
        | TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => Some(TrayAction::Show),
        _ => None,
    }
}

pub fn is_launch_at_login_enabled() -> Result<bool> {
    auto_launch()?.is_enabled().context("read launch at login")
}

pub fn set_launch_at_login(enabled: bool) -> Result<()> {
    let auto_launch = auto_launch()?;
    if enabled {
        auto_launch.enable().context("enable launch at login")
    } else {
        auto_launch.disable().context("disable launch at login")
    }
}

fn tray_icon() -> Result<Icon> {
    let icon = eframe::icon_data::from_png_bytes(ICON_BYTES).context("decode tray icon")?;
    Icon::from_rgba(icon.rgba, icon.width, icon.height).context("create tray icon image")
}

fn auto_launch() -> Result<AutoLaunch> {
    let app_path = std::env::current_exe()
        .context("resolve current executable")?
        .to_string_lossy()
        .into_owned();
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(&app_path)
        .set_args(&[AUTOSTART_ARG]);

    #[cfg(target_os = "linux")]
    builder.set_linux_launch_mode(auto_launch::LinuxLaunchMode::XdgAutostart);

    #[cfg(target_os = "macos")]
    {
        builder
            .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
            .set_bundle_identifiers(&[APP_ID]);
    }

    #[cfg(target_os = "windows")]
    builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);

    builder.build().context("create launch at login handle")
}

pub fn has_minimized_arg() -> bool {
    std::env::args_os().any(|arg| arg == AUTOSTART_ARG)
}

pub fn app_icon_bytes() -> &'static [u8] {
    ICON_BYTES
}

#[allow(dead_code)]
pub fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("resolve current executable")
}
