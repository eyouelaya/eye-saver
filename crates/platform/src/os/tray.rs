//! The menu bar / notification area item.
//!
//! macOS has room for text beside the status item, so EyeSaver shows the countdown as
//! plain text there and no icon at all - a blank image would just be visual noise next to
//! a number that already says everything. Windows has no such room: the notification area
//! is icon-plus-tooltip only, so the countdown is drawn *into* the icon as digits, computed
//! by `eye_saver_core::icon` and handed over as raw pixels - there is no image file to ship
//! or to go missing.

use eye_saver_core::engine::{IconKind, MenuModel};
#[cfg(not(target_os = "macos"))]
use eye_saver_core::icon;
use eye_saver_core::ports::{HostResult, Tray};
#[cfg(not(target_os = "macos"))]
use tray_icon::Icon;
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

/// A menu item the user can choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    /// Start the look-away break now.
    TakeBreakNow,
    /// Declare this interval finished and start the next one.
    ResetCycle,
    /// Pause, or resume if already paused.
    TogglePause,
    /// Show the settings window.
    OpenSettings,
    /// Quit.
    Quit,
}

/// The tray item and its menu.
pub struct TrayAdapter {
    tray: TrayIcon,
    header: MenuItem,
    pause: MenuItem,
    choices: Vec<(MenuId, MenuChoice)>,
    #[cfg(not(target_os = "macos"))]
    icon_size: u32,
    #[cfg(not(target_os = "macos"))]
    drawn: Option<(IconKind, Option<u16>)>,
}

impl TrayAdapter {
    /// Creates the tray item.
    ///
    /// # Errors
    ///
    /// If the platform refuses to create the item or its menu, which on macOS means this
    /// was not called from the main thread after the event loop started.
    pub fn new(initial: &MenuModel) -> Result<Self, String> {
        let menu = Menu::new();
        let header = MenuItem::new(&initial.header, false, None);
        let break_now = MenuItem::new("Take a break now", true, None);
        let reset = MenuItem::new("Reset the timer", true, None);
        let pause = MenuItem::new(initial.pause_label, true, None);
        let settings = MenuItem::new("Settings...", true, None);
        let quit = MenuItem::new("Quit EyeSaver", true, None);

        let separator = PredefinedMenuItem::separator();
        for item in [
            &header as &dyn tray_icon::menu::IsMenuItem,
            &separator,
            &break_now,
            &reset,
            &pause,
            &separator,
            &settings,
            &quit,
        ] {
            menu.append(item)
                .map_err(|e| format!("building the tray menu: {e}"))?;
        }

        let choices = vec![
            (break_now.id().clone(), MenuChoice::TakeBreakNow),
            (reset.id().clone(), MenuChoice::ResetCycle),
            (pause.id().clone(), MenuChoice::TogglePause),
            (settings.id().clone(), MenuChoice::OpenSettings),
            (quit.id().clone(), MenuChoice::Quit),
        ];

        let icon_size = super::tray_icon_size();
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(eye_saver_core::APP_NAME);

        // On macOS the status item shows the countdown as text; no icon is needed, and an
        // empty or placeholder image would just be clutter next to the number. Windows has
        // no text label, so its icon *is* the countdown and must exist from the start.
        #[cfg(target_os = "macos")]
        let builder = builder;
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(render(IconKind::Work, icon_size, None)?);

        let tray = builder
            .build()
            .map_err(|e| format!("creating the tray icon: {e}"))?;

        #[cfg(target_os = "macos")]
        {
            let _ = icon_size;
            Ok(Self {
                tray,
                header,
                pause,
                choices,
            })
        }
        #[cfg(not(target_os = "macos"))]
        Ok(Self {
            tray,
            header,
            pause,
            choices,
            icon_size,
            drawn: Some((IconKind::Work, None)),
        })
    }

    /// Which action a clicked menu item represents, if any.
    #[must_use]
    pub fn choice(&self, id: &MenuId) -> Option<MenuChoice> {
        self.choices
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, choice)| *choice)
    }
}

/// Draws the artwork at the size the host wants.
///
/// Only used off macOS: the status item there is text-only and has no icon to draw.
#[cfg(not(target_os = "macos"))]
fn render(kind: IconKind, size: u32, badge: Option<u16>) -> Result<Icon, String> {
    let image = icon::render(kind, size, icon::INK_TRAY, badge);
    Icon::from_rgba(image.into_bytes(), size, size).map_err(|e| format!("drawing the icon: {e}"))
}

impl Tray for TrayAdapter {
    fn set_label(&mut self, label: &str) -> HostResult {
        // Only macOS has room for text beside the icon; elsewhere the engine sends an
        // empty label and the countdown goes into the icon instead.
        #[cfg(target_os = "macos")]
        self.tray.set_title(Some(label));
        #[cfg(not(target_os = "macos"))]
        let _ = label;
        Ok(())
    }

    fn set_tooltip(&mut self, tooltip: &str) -> HostResult {
        self.tray
            .set_tooltip(Some(tooltip))
            .map_err(|e| format!("setting the tooltip: {e}"))
    }

    // The status item is text-only on macOS, so there is nothing to draw there: the
    // engine still emits this effect (the countdown text covers the information
    // instead), and it is simply a no-op on that platform.
    #[cfg(target_os = "macos")]
    fn set_icon(&mut self, kind: IconKind, badge: Option<u16>) -> HostResult {
        let _ = (kind, badge);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn set_icon(&mut self, kind: IconKind, badge: Option<u16>) -> HostResult {
        if self.drawn == Some((kind, badge)) {
            return Ok(());
        }
        let icon = render(kind, self.icon_size, badge)?;
        self.tray
            .set_icon(Some(icon))
            .map_err(|e| format!("setting the icon: {e}"))?;
        self.drawn = Some((kind, badge));
        Ok(())
    }

    fn set_menu(&mut self, menu: &MenuModel) -> HostResult {
        self.header.set_text(&menu.header);
        self.pause.set_text(menu.pause_label);
        Ok(())
    }
}
