#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod diagnostics;
#[cfg(test)]
mod diagnostics_tests;
mod diagnostics_ui;
mod dxf_import;
mod geometry;
mod layout;
mod measurement;
mod measurement_ui;
#[cfg(test)]
mod test_fixtures;

use app::DxfCanvasApp;
use eframe::egui;

fn main() -> eframe::Result {
  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_title(concat!("DXF Холст v", env!("CARGO_PKG_VERSION")))
      .with_icon(egui::IconData {
        rgba: app_icon::rgba(64),
        width: 64,
        height: 64,
      })
      .with_inner_size([1280.0, 820.0])
      .with_min_inner_size([760.0, 520.0]),
    centered: true,
    ..Default::default()
  };

  eframe::run_native(
    "DXF Холст",
    options,
    Box::new(|context| Ok(Box::new(DxfCanvasApp::new(context)))),
  )
}
