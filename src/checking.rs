use std::sync::{
  Arc,
  atomic::{AtomicUsize, Ordering},
  mpsc::{self, Receiver, TryRecvError},
};

use eframe::egui;

use crate::{
  cad_scene::Appearance,
  diagnostics::{DiagnosticReport, analyze},
  geometry::DrawingItem,
};

type CheckResult = Result<Vec<DiagnosticReport>, String>;

#[derive(Default)]
pub struct CheckJob {
  requested: bool,
  result: Option<Receiver<CheckResult>>,
  completed: Arc<AtomicUsize>,
  total: usize,
}

impl CheckJob {
  pub fn request(&mut self) {
    if !self.is_busy() {
      self.requested = true;
    }
  }

  pub fn is_busy(&self) -> bool {
    self.requested || self.result.is_some()
  }

  pub fn poll(&mut self, context: &egui::Context, items: &[DrawingItem]) -> Option<CheckResult> {
    if let Some(receiver) = &self.result {
      let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err("Проверка чертежа неожиданно завершилась.".into()),
      };
      self.result = None;
      return Some(result);
    }
    if !self.requested {
      return None;
    }
    self.requested = false;
    self.total = items.len();
    self.completed.store(0, Ordering::Relaxed);
    let snapshots: Vec<_> = items.iter().map(snapshot).collect();
    let completed = self.completed.clone();
    let wake = context.clone();
    let (sender, receiver) = mpsc::channel();
    match std::thread::Builder::new()
      .name("dxf-check".into())
      .spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
          snapshots
            .iter()
            .map(|item| {
              let report = analyze(item);
              completed.fetch_add(1, Ordering::Relaxed);
              wake.request_repaint();
              report
            })
            .collect()
        }))
        .map_err(|_| "Не удалось завершить проверку геометрии. Исходный чертёж не изменён.".into());
        let _ = sender.send(result);
        wake.request_repaint();
      }) {
      Ok(_) => {
        self.result = Some(receiver);
        None
      }
      Err(error) => Some(Err(format!("Не удалось запустить проверку: {error}"))),
    }
  }

  pub fn show(&self, context: &egui::Context) {
    if !self.is_busy() {
      return;
    }
    egui::Modal::new(egui::Id::new("dxf_checking"))
      .backdrop_color(egui::Color32::from_black_alpha(45))
      .show(context, |ui| {
        ui.set_width(350.0);
        ui.horizontal(|ui| {
          ui.add(egui::Spinner::new().size(28.0));
          ui.heading("Проверка геометрии…");
        });
        ui.label(format!(
          "Проверено файлов: {} / {}",
          self.completed.load(Ordering::Relaxed),
          self.total
        ));
        ui.label("Чертежи не изменяются. На больших планах проверка может занять некоторое время.");
      });
    context.request_repaint_after(std::time::Duration::from_millis(16));
  }
}

fn snapshot(item: &DrawingItem) -> DrawingItem {
  // Анализу нужны исходные кривые и видимость слоёв, но не текст, заливки и экранные сетки.
  DrawingItem {
    rotation: item.rotation,
    appearance: Appearance {
      layers: item.appearance.layers.clone(),
      styles: item.appearance.styles.clone(),
      ..Default::default()
    },
    primitives: item.primitives.clone(),
    path: item.path.clone(),
    name: item.name.clone(),
    bounds: item.bounds,
    offset: item.offset,
    scale: item.scale,
    units: item.units,
    unsupported_entities: item.unsupported_entities,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn background_check_returns_the_same_findings_and_cannot_start_twice() {
    let mut item = crate::dxf_import::load_dxf(
      &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/measurement_demo.dxf"),
    )
    .unwrap();
    item.appearance.styles[0].visible = false;
    let expected = analyze(&item);
    let mut job = CheckJob::default();
    let context = egui::Context::default();
    job.request();
    assert!(job.is_busy());
    assert!(job.poll(&context, std::slice::from_ref(&item)).is_none());
    job.request();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
      if let Some(result) = job.poll(&context, std::slice::from_ref(&item)) {
        let reports = result.unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(format!("{:?}", reports[0]), format!("{expected:?}"));
        assert!(!job.is_busy());
        break;
      }
      assert!(std::time::Instant::now() < deadline);
      std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!item.appearance.styles[0].visible);
  }

  #[test]
  fn disconnected_worker_clears_busy_state_and_reports_an_error() {
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    let mut job = CheckJob {
      result: Some(receiver),
      ..Default::default()
    };
    assert!(job.poll(&egui::Context::default(), &[]).unwrap().is_err());
    assert!(!job.is_busy());
  }
}
