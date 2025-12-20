use egui::WidgetText;
use egui_toast::{Toast, ToastKind, ToastOptions};

use crate::MyApp;

impl MyApp {
    pub fn show_error_toast(&mut self, text: impl Into<WidgetText>) {
        self.toasts.add(Toast {
            text: text.into(),
            kind: ToastKind::Error,
            options: ToastOptions::default()
                .duration_in_seconds(3.)
                .show_progress(true),
            ..Default::default()
        });
    }
}
