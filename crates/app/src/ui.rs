//! The settings page.
//!
//! The only window EyeSaver has. It edits a draft copy of the settings and reports when
//! the user asks to keep or discard it, so that the engine is handed one complete change
//! rather than a change per keystroke - which matters, because changing the interval
//! restarts the countdown.

use eye_saver_core::settings::{BREAK_SECS, IDLE_SECS, Settings, TrayDisplay, WORK_SECS};

/// What the user did with the page this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Action {
    /// Apply the draft.
    pub save: bool,
    /// Throw the draft away.
    pub revert: bool,
    /// Close the window.
    pub close: bool,
}

/// The dropdown label for a display mode.
///
/// macOS has no tray icon any more - the countdown is shown as text instead - so there is
/// nothing left for `TrayDisplay::IconOnly` to show there: it means a blank status item,
/// not an icon. Everywhere else (Windows, whose notification area has no text label) the
/// mode still draws a plain glyph, so "Icon only" stays accurate.
fn display_label(display: TrayDisplay) -> &'static str {
    if cfg!(target_os = "macos") && display == TrayDisplay::IconOnly {
        "No icon"
    } else {
        display.label()
    }
}

/// Draws the settings page into `ui`.
pub fn settings_page(ui: &mut egui::Ui, draft: &mut Settings, saved: &Settings) -> Action {
    let mut action = Action::default();

    ui.add_space(4.0);
    ui.heading("EyeSaver");
    ui.label(
        egui::RichText::new(
            "Every 20 minutes, look at something about 20 feet away for 20 seconds.",
        )
        .weak(),
    );
    ui.add_space(12.0);

    egui::Grid::new("settings")
        .num_columns(2)
        .spacing([12.0, 10.0])
        .show(ui, |ui| {
            ui.label("Remind me every");
            minutes_row(ui, &mut draft.work_interval_secs, WORK_SECS);
            ui.end_row();

            ui.label("Look away for");
            seconds_row(ui, &mut draft.break_duration_secs, BREAK_SECS);
            ui.end_row();

            ui.label("Count time away as a break after");
            seconds_row(ui, &mut draft.idle_credit_secs, IDLE_SECS);
            ui.end_row();

            ui.label(if cfg!(target_os = "macos") {
                "Menu bar shows"
            } else {
                "Tray icon shows"
            });
            egui::ComboBox::from_id_salt("tray-display")
                .selected_text(display_label(draft.tray_display))
                .show_ui(ui, |ui| {
                    for display in TrayDisplay::ALL {
                        ui.selectable_value(
                            &mut draft.tray_display,
                            display,
                            display_label(display),
                        );
                    }
                });
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.checkbox(&mut draft.start_at_login, "Start EyeSaver when I log in");
    ui.checkbox(
        &mut draft.notification_sound,
        "Play a sound with the reminder",
    );

    // Always laid out, at the same size, whether or not it applies: reserving this
    // space unconditionally is what keeps the Save/Revert row below from jumping up and
    // down every time the display mode is switched in or out of "Countdown".
    ui.add_space(6.0);
    ui.scope(|ui| {
        if draft.tray_display != TrayDisplay::MmSs {
            ui.set_invisible();
        }
        ui.label(
            egui::RichText::new(format!(
                "A live countdown updates once a second. On battery, \"minutes\" or \
                 \"{}\" wakes the app far less often.",
                display_label(TrayDisplay::IconOnly)
            ))
            .weak()
            .small(),
        );
    });

    ui.add_space(14.0);
    ui.separator();
    ui.add_space(8.0);

    let dirty = draft != saved;
    ui.horizontal(|ui| {
        if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
            action.save = true;
        }
        if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
            action.revert = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                action.close = true;
            }
            if dirty {
                ui.label(egui::RichText::new("unsaved changes").weak().small());
            }
        });
    });

    action
}

/// A whole-minute field, edited in minutes but stored in seconds.
fn minutes_row(ui: &mut egui::Ui, secs: &mut u64, limits: (u64, u64)) {
    let mut minutes = (*secs / 60).max(limits.0 / 60);
    let range = (limits.0 / 60)..=(limits.1 / 60);
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut minutes)
                .range(range.clone())
                .speed(0.2),
        );
        ui.label(if minutes == 1 { "minute" } else { "minutes" });
    });
    *secs = (minutes * 60).clamp(limits.0, limits.1);
}

/// A whole-second field.
fn seconds_row(ui: &mut egui::Ui, secs: &mut u64, limits: (u64, u64)) {
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(secs)
                .range(limits.0..=limits.1)
                .speed(0.5),
        );
        ui.label(if *secs == 1 { "second" } else { "seconds" });
    });
}
