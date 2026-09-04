use std::{
  collections::{HashSet, VecDeque},
  path::{Path, PathBuf},
  sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, TryRecvError},
  },
};

use eframe::egui;

use crate::{
  diagnostics::{DiagnosticReport, analyze},
  dwg_import::{is_dwg, load_drawing},
  geometry::DrawingItem,
};

pub struct LoadedDrawing {
  pub item: DrawingItem,
  pub report: Option<DiagnosticReport>,
}

type ImportResult = Result<Box<LoadedDrawing>, String>;

struct ActiveImport {
  path: PathBuf,
  result: Receiver<ImportResult>,
  phase: Arc<Mutex<String>>,
}

#[derive(Default)]
pub struct ImportQueue {
  pending: VecDeque<PathBuf>,
  active: Option<ActiveImport>,
  total: usize,
  completed: usize,
}

impl ImportQueue {
  pub fn is_busy(&self) -> bool {
    self.active.is_some() || !self.pending.is_empty()
  }

  pub fn enqueue(&mut self, paths: Vec<PathBuf>, items: &[DrawingItem]) -> Vec<String> {
    if !self.is_busy() {
      self.total = 0;
      self.completed = 0;
    }
    let mut existing: HashSet<_> = items
      .iter()
      .map(|item| path_key(&item.path))
      .chain(self.pending.iter().map(|path| path_key(path)))
      .chain(self.active.iter().map(|active| path_key(&active.path)))
      .collect();
    let mut errors = Vec::new();
    for path in paths {
      if !is_supported_drawing(&path) {
        errors.push(format!(
          "{}: поддерживаются только файлы .dxf и .dwg",
          path.display()
        ));
      } else if existing.insert(path_key(&path)) {
        self.pending.push_back(path);
        self.total += 1;
      }
    }
    errors
  }

  pub fn progress(&self) -> Option<(&Path, usize, usize)> {
    let path = self
      .active
      .as_ref()
      .map(|job| &job.path)
      .or(self.pending.front())?;
    Some((path, self.completed + 1, self.total))
  }

  fn phase(&self) -> String {
    self
      .active
      .as_ref()
      .and_then(|active| active.phase.lock().ok().map(|text| text.clone()))
      .filter(|text| !text.is_empty())
      .unwrap_or_else(|| "Подготовка загрузки…".into())
  }

  pub fn poll(&mut self, context: &egui::Context, diagnostics: bool) -> Option<ImportResult> {
    if let Some(active) = &self.active {
      let result = match active.result.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(format!(
          "{}: фоновая загрузка неожиданно завершилась",
          active.path.display()
        )),
      };
      self.active = None;
      self.completed += 1;
      context.request_repaint();
      return Some(result);
    }
    let path = self.pending.pop_front()?;
    let worker_path = path.clone();
    let wake = context.clone();
    let (sender, result) = mpsc::channel();
    let phase = Arc::new(Mutex::new(String::new()));
    let worker_phase = phase.clone();
    // Один файл за раз: большие DXF не загружаются одновременно и не умножают расход памяти.
    match std::thread::Builder::new()
      .name("dxf-import".into())
      .spawn(move || {
        let update_phase = |text: &str| {
          if let Ok(mut phase) = worker_phase.lock() {
            *phase = text.to_owned();
          }
          wake.request_repaint();
        };
        let loaded = load_drawing(&worker_path, update_phase)
          .map(|item| {
            let report = diagnostics.then(|| {
              update_phase("Проверка геометрии чертежа…");
              analyze(&item)
            });
            Box::new(LoadedDrawing { item, report })
          })
          .map_err(|error| format!("{}: {error}", worker_path.display()));
        let _ = sender.send(loaded);
        wake.request_repaint();
      }) {
      Ok(_) => {
        self.active = Some(ActiveImport {
          path,
          result,
          phase,
        });
        None
      }
      Err(error) => {
        self.completed += 1;
        context.request_repaint();
        Some(Err(format!(
          "{}: не удалось запустить загрузку: {error}",
          path.display()
        )))
      }
    }
  }
}

pub fn show_loading(context: &egui::Context, queue: &ImportQueue) {
  let Some((path, index, total)) = queue.progress() else {
    return;
  };
  egui::Modal::new(egui::Id::new("dxf_loading"))
    .backdrop_color(egui::Color32::from_black_alpha(45))
    .show(context, |ui| {
      ui.set_width(360.0_f32.min((context.content_rect().width() - 50.0).max(100.0)));
      ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(28.0));
        ui.heading(if is_dwg(path) {
          "Загрузка DWG…"
        } else {
          "Загрузка DXF…"
        });
      });
      ui.add_space(8.0);
      ui.label(format!("Файл {index} из {total}"));
      ui.add(
        egui::Label::new(
          path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy(),
        )
        .wrap(),
      );
      ui.add_space(6.0);
      ui.label(egui::RichText::new(queue.phase()).weak());
    });
  // Обновление также позволяет обнаружить аварийно завершившийся поток без сигнала от него.
  context.request_repaint_after(std::time::Duration::from_millis(16));
}

pub fn is_dxf(path: &Path) -> bool {
  path
    .extension()
    .and_then(|extension| extension.to_str())
    .is_some_and(|extension| extension.eq_ignore_ascii_case("dxf"))
}

pub fn is_supported_drawing(path: &Path) -> bool {
  is_dxf(path) || is_dwg(path)
}

fn path_key(path: &Path) -> PathBuf {
  // Без обращения к диску: сетевой путь не должен задерживать первый кадр со спиннером.
  let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
  #[cfg(windows)]
  let absolute = PathBuf::from(absolute.to_string_lossy().to_lowercase());
  absolute
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mixed_dwg_dxf_queue_accepts_both_formats_and_tracks_conversion_phase() {
    let mut queue = ImportQueue::default();
    assert!(
      queue
        .enqueue(vec!["первый.DWG".into(), "второй.dxf".into()], &[])
        .is_empty()
    );
    assert_eq!(queue.progress(), Some((Path::new("первый.DWG"), 1, 2)));
    let (_sender, result) = mpsc::channel();
    queue.active = Some(ActiveImport {
      path: queue.pending.pop_front().unwrap(),
      result,
      phase: Arc::new(Mutex::new("Преобразование DWG → DXF…".into())),
    });
    assert_eq!(queue.phase(), "Преобразование DWG → DXF…");
    assert!(is_supported_drawing(Path::new("чертёж.DWG")));
    assert!(!is_supported_drawing(Path::new("чертёж.bak")));
  }

  #[test]
  fn queued_and_active_files_are_not_added_twice() {
    let mut queue = ImportQueue::default();
    let path = PathBuf::from("первая.dxf");
    let errors = queue.enqueue(
      vec![path.clone(), path.clone(), "не чертёж.txt".into()],
      &[],
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(queue.progress(), Some((path.as_path(), 1, 1)));
    let (_sender, result) = mpsc::channel();
    queue.active = Some(ActiveImport {
      path: queue.pending.pop_front().unwrap(),
      result,
      phase: Arc::default(),
    });
    queue.enqueue(vec![path, "вторая.DXF".into()], &[]);
    assert_eq!(queue.total, 2);
    assert_eq!(queue.pending.len(), 1);
  }

  #[test]
  fn waiting_is_non_blocking_and_failure_keeps_the_next_file() {
    let context = egui::Context::default();
    let mut queue = ImportQueue::default();
    queue.enqueue(vec!["первая.dxf".into(), "вторая.dxf".into()], &[]);
    let (sender, result) = mpsc::channel();
    queue.active = Some(ActiveImport {
      path: queue.pending.pop_front().unwrap(),
      result,
      phase: Arc::default(),
    });
    // Отправитель жив и ещё не отправил результат: опрос не должен ждать чтения DXF.
    assert!(queue.poll(&context, false).is_none());
    assert!(queue.is_busy());
    sender.send(Err("Повреждённый DXF".into())).unwrap();
    assert!(queue.poll(&context, false).unwrap().is_err());
    assert_eq!(queue.progress(), Some((Path::new("вторая.dxf"), 2, 2)));
    assert!(queue.is_busy());
  }

  #[test]
  fn disconnected_worker_clears_spinner_and_allows_retry() {
    let context = egui::Context::default();
    let mut queue = ImportQueue::default();
    queue.enqueue(vec!["повтор.dxf".into()], &[]);
    let (sender, result) = mpsc::channel();
    queue.active = Some(ActiveImport {
      path: queue.pending.pop_front().unwrap(),
      result,
      phase: Arc::default(),
    });
    drop(sender);
    assert!(queue.poll(&context, false).unwrap().is_err());
    assert!(!queue.is_busy());
    assert!(queue.progress().is_none());
    queue.enqueue(vec!["повтор.dxf".into()], &[]);
    assert_eq!(queue.progress(), Some((Path::new("повтор.dxf"), 1, 1)));
  }

  #[test]
  fn real_worker_reports_missing_file_and_finishes() {
    let context = egui::Context::default();
    let mut queue = ImportQueue::default();
    queue.enqueue(
      vec![std::env::temp_dir().join(format!("missing-{}-dxf-canvas.dxf", std::process::id()))],
      &[],
    );
    assert!(queue.poll(&context, false).is_none());
    let result = queue
      .active
      .as_ref()
      .unwrap()
      .result
      .recv_timeout(std::time::Duration::from_secs(5))
      .unwrap();
    assert!(result.is_err());
  }

  #[test]
  fn modal_shows_file_name_and_counter_only_while_busy() {
    let context = egui::Context::default();
    let mut queue = ImportQueue::default();
    queue.enqueue(vec!["длинное имя файла.dxf".into()], &[]);
    for _ in 0..2 {
      let mut output = context.run_ui(Default::default(), |_| show_loading(&context, &queue));
      output.textures_delta.clear();
      let text = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
          egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
          _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
      if !text.is_empty() {
        assert!(text.contains("Загрузка DXF"));
        assert!(text.contains("Файл 1 из 1"));
        assert!(text.contains("длинное имя файла.dxf"));
        return;
      }
    }
    panic!("Окно загрузки не отрисовано");
  }
}
