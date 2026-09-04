use std::{path::PathBuf, sync::Arc};

use eframe::egui::{
  self, Align2, Color32, FontFamily, FontId, KeyboardShortcut, Modifiers, PointerButton, Pos2,
  Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};

use crate::{
  diagnostics::DiagnosticsState,
  diagnostics_ui::{paint_report, paint_selected_finding, show_file_report, show_legend},
  geometry::{Bounds, DrawingItem, Point, ViewTransform},
  layout::arrange,
  loading::{ImportQueue, is_supported_drawing, show_loading},
  measurement::{MeasurementState, Tool},
  measurement_ui::{paint_dimension, paint_round_highlight, paint_snap},
};

const CANVAS_PADDING: f32 = 54.0;
const MIN_ZOOM: f32 = 0.00001;
const MAX_ZOOM: f32 = 100_000.0;
const MIN_ITEM_SCALE: f64 = 0.05;
const MAX_ITEM_SCALE: f64 = 20.0;
const RESIZE_HANDLE_SIZE: f32 = 9.0;
const DEFAULT_LABEL_FONT_SIZE: f32 = 16.0;

#[derive(Clone, Copy, Debug)]
enum ResizeCorner {
  TopLeft,
  TopRight,
  BottomLeft,
  BottomRight,
}

impl ResizeCorner {
  fn opposite_local(self, bounds: Bounds) -> Point {
    match self {
      Self::TopLeft => Point::new(bounds.max.x, bounds.min.y),
      Self::TopRight => Point::new(bounds.min.x, bounds.min.y),
      Self::BottomLeft => Point::new(bounds.max.x, bounds.max.y),
      Self::BottomRight => Point::new(bounds.min.x, bounds.max.y),
    }
  }

  fn cursor(self) -> egui::CursorIcon {
    match self {
      Self::TopLeft | Self::BottomRight => egui::CursorIcon::ResizeNwSe,
      Self::TopRight | Self::BottomLeft => egui::CursorIcon::ResizeNeSw,
    }
  }
}

#[derive(Clone, Copy, Debug)]
enum CanvasInteraction {
  Pan,
  MoveItem(usize),
  ScaleItem {
    index: usize,
    corner: ResizeCorner,
    anchor_local: Point,
    anchor_world: Point,
    start_distance: f64,
    start_scale: f64,
  },
}

pub struct DxfCanvasApp {
  items: Vec<DrawingItem>,
  errors: Vec<String>,
  world_bounds: Option<Bounds>,
  view_center: Point,
  zoom: f32,
  needs_layout: bool,
  needs_fit: bool,
  selected_item: Option<usize>,
  interaction: Option<CanvasInteraction>,
  label_font_size: f32,
  measurements: MeasurementState,
  diagnostics: DiagnosticsState,
  layer_filter: String,
  imports: ImportQueue,
}

impl DxfCanvasApp {
  pub fn new(context: &eframe::CreationContext<'_>) -> Self {
    configure_fonts_and_style(&context.egui_ctx);
    let mut app = Self {
      items: Vec::new(),
      errors: Vec::new(),
      world_bounds: None,
      view_center: Point::default(),
      zoom: 1.0,
      needs_layout: false,
      needs_fit: false,
      selected_item: None,
      interaction: None,
      label_font_size: DEFAULT_LABEL_FONT_SIZE,
      measurements: MeasurementState::default(),
      diagnostics: DiagnosticsState::default(),
      layer_filter: String::new(),
      imports: ImportQueue::default(),
    };

    let startup_files: Vec<_> = std::env::args_os()
      .skip(1)
      .map(PathBuf::from)
      .filter(|path| is_supported_drawing(path))
      .collect();
    app.add_paths(startup_files);
    app
  }

  fn choose_files(&mut self) {
    if let Some(paths) = rfd::FileDialog::new()
      .set_title("Выберите DWG- или DXF-файлы")
      .add_filter("Чертежи DWG и DXF", &["dxf", "dwg", "DXF", "DWG"])
      .pick_files()
    {
      self.add_paths(paths);
    }
  }

  fn add_paths(&mut self, paths: Vec<PathBuf>) {
    self.errors.extend(self.imports.enqueue(paths, &self.items));
    if self.imports.is_busy() {
      self.interaction = None;
    }
  }

  fn poll_imports(&mut self, context: &egui::Context) {
    if let Some(result) = self.imports.poll(context, self.diagnostics.enabled) {
      match result {
        Ok(loaded) => {
          if let Some(report) = loaded.report {
            self.diagnostics.reports.push(report);
          }
          self.diagnostics.clear_selection();
          self.items.push(loaded.item);
          self.needs_layout = true;
          self.needs_fit = true;
        }
        Err(error) => self.errors.push(error),
      }
    }
  }

  fn perform_layout(&mut self, canvas_rect: Rect) {
    let aspect = (canvas_rect.width() / canvas_rect.height().max(1.0)) as f64;
    self.world_bounds = arrange(&mut self.items, aspect);
    self.needs_layout = false;
  }

  fn recalculate_world_bounds(&mut self) {
    let mut bounds = Bounds::empty();
    for item in &self.items {
      bounds.include_bounds(item.placed_bounds());
    }
    self.world_bounds = bounds.is_valid().then_some(bounds);
  }

  fn fit_all(&mut self, canvas_rect: Rect) {
    let Some(bounds) = self.world_bounds else {
      return;
    };
    let usable_width = (canvas_rect.width() - CANVAS_PADDING * 2.0).max(1.0);
    let usable_height = (canvas_rect.height() - CANVAS_PADDING * 2.0).max(1.0);
    let scale_x = usable_width / bounds.width().max(1.0e-9) as f32;
    let scale_y = usable_height / bounds.height().max(1.0e-9) as f32;
    self.zoom = scale_x.min(scale_y).clamp(MIN_ZOOM, MAX_ZOOM);
    self.view_center = bounds.center();
    self.needs_fit = false;
  }

  fn transform(&self, canvas_rect: Rect) -> ViewTransform {
    ViewTransform {
      scale: self.zoom,
      origin: egui::pos2(
        canvas_rect.center().x - self.view_center.x as f32 * self.zoom,
        canvas_rect.center().y + self.view_center.y as f32 * self.zoom,
      ),
    }
  }

  fn focus_requested_finding(&mut self, canvas_rect: Rect) {
    let Some(selection) = self.diagnostics.take_focus_request() else {
      return;
    };
    let Some(item) = self.items.get(selection.item) else {
      return;
    };
    let Some(finding) = self
      .diagnostics
      .reports
      .get(selection.item)
      .and_then(|report| report.findings.get(selection.finding))
    else {
      return;
    };
    let Some(bounds) = finding.marker.focus_bounds(item) else {
      return;
    };
    self.view_center = bounds.center();
    self.zoom = ((canvas_rect.width() * 0.65 / bounds.width().max(1.0e-9) as f32)
      .min(canvas_rect.height() * 0.65 / bounds.height().max(1.0e-9) as f32))
    .clamp(MIN_ZOOM, MAX_ZOOM);
    self.selected_item = Some(selection.item);
    self.interaction = None;
    self.needs_fit = false;
  }

  fn handle_canvas_input(&mut self, ui: &egui::Ui, response: &egui::Response, canvas_rect: Rect) {
    let transform = self.transform(canvas_rect);
    let (
      pointer,
      primary_pressed,
      primary_down,
      primary_released,
      middle_pressed,
      middle_down,
      middle_released,
      delta,
      scroll,
      ctrl,
    ) = ui.input(|input| {
      (
        input.pointer.interact_pos(),
        input.pointer.button_pressed(PointerButton::Primary),
        input.pointer.button_down(PointerButton::Primary),
        input.pointer.button_released(PointerButton::Primary),
        input.pointer.button_pressed(PointerButton::Middle),
        input.pointer.button_down(PointerButton::Middle),
        input.pointer.button_released(PointerButton::Middle),
        input.pointer.delta(),
        input.smooth_scroll_delta.y,
        input.modifiers.ctrl,
      )
    });

    if self.measurements.tool != Tool::Select {
      if response.hovered() && middle_pressed {
        self.interaction = Some(CanvasInteraction::Pan);
      }
      if middle_down && matches!(self.interaction, Some(CanvasInteraction::Pan)) {
        self.view_center.x -= delta.x as f64 / self.zoom as f64;
        self.view_center.y += delta.y as f64 / self.zoom as f64;
      }
      if middle_released {
        self.interaction = None;
      }
      if response.hovered() {
        if let Some(pointer) = pointer {
          if scroll.abs() > 0.01 {
            self.zoom_at(pointer, canvas_rect, (scroll * 0.0025).exp());
          }
          if primary_pressed {
            self
              .measurements
              .click(&self.items, self.transform(canvas_rect), pointer);
          }
        }
        if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
          self.measurements.cancel();
        }
        ui.ctx().set_cursor_icon(if middle_down {
          egui::CursorIcon::Grabbing
        } else {
          egui::CursorIcon::Crosshair
        });
      }
      return;
    }

    if response.hovered() && middle_pressed {
      self.interaction = Some(CanvasInteraction::Pan);
    } else if response.hovered()
      && primary_pressed
      && let Some(pointer) = pointer
    {
      let handle = self.selected_item.and_then(|index| {
        resize_handle_at(&self.items[index], transform, pointer).map(|corner| (index, corner))
      });
      if let Some((index, corner)) = handle {
        let item = &self.items[index];
        let anchor_local = corner.opposite_local(item.bounds);
        let anchor_world = item.world_point(anchor_local);
        let pointer_world = transform.screen_to_world(pointer);
        self.interaction = Some(CanvasInteraction::ScaleItem {
          index,
          corner,
          anchor_local,
          anchor_world,
          start_distance: distance(pointer_world, anchor_world).max(1.0e-9),
          start_scale: item.scale,
        });
      } else if let Some(index) = hit_test_items(&self.items, transform, pointer) {
        self.selected_item = Some(index);
        self.interaction = Some(CanvasInteraction::MoveItem(index));
      } else {
        self.selected_item = None;
        self.interaction = Some(CanvasInteraction::Pan);
      }
    }

    let mut item_changed = false;
    if (primary_down || middle_down) && delta != Vec2::ZERO {
      match self.interaction {
        Some(CanvasInteraction::Pan) => {
          self.view_center.x -= delta.x as f64 / self.zoom as f64;
          self.view_center.y += delta.y as f64 / self.zoom as f64;
        }
        Some(CanvasInteraction::MoveItem(index)) if primary_down => {
          if let Some(item) = self.items.get_mut(index) {
            move_item_by_screen_delta(item, delta, self.zoom);
            item_changed = true;
          }
        }
        Some(CanvasInteraction::ScaleItem {
          index,
          anchor_local,
          anchor_world,
          start_distance,
          start_scale,
          ..
        }) if primary_down => {
          if let (Some(item), Some(pointer)) = (self.items.get_mut(index), pointer) {
            let pointer_world = transform.screen_to_world(pointer);
            let scale = (start_scale * distance(pointer_world, anchor_world) / start_distance)
              .clamp(MIN_ITEM_SCALE, MAX_ITEM_SCALE);
            item.set_scale_keeping_anchor(scale, anchor_local, anchor_world);
            item_changed = true;
          }
        }
        _ => {}
      }
    }

    if response.hovered() && scroll.abs() > 0.01 {
      let pointer = pointer.unwrap_or(canvas_rect.center());
      if ctrl {
        let index = hit_test_items(&self.items, transform, pointer).or(self.selected_item);
        if let Some(index) = index {
          self.selected_item = Some(index);
          let world_anchor = transform.screen_to_world(pointer);
          let item = &mut self.items[index];
          let local_anchor = item.local_point(world_anchor);
          let scale =
            (item.scale * (scroll as f64 * 0.0025).exp()).clamp(MIN_ITEM_SCALE, MAX_ITEM_SCALE);
          item.set_scale_keeping_anchor(scale, local_anchor, world_anchor);
          item_changed = true;
        }
      } else {
        self.zoom_at(pointer, canvas_rect, (scroll * 0.0025).exp());
      }
    }

    if primary_released || middle_released {
      self.interaction = None;
    }
    if item_changed {
      self.recalculate_world_bounds();
    }

    let cursor = match self.interaction {
      Some(CanvasInteraction::Pan) if primary_down || middle_down => egui::CursorIcon::Grabbing,
      Some(CanvasInteraction::MoveItem(_)) if primary_down => egui::CursorIcon::Grabbing,
      Some(CanvasInteraction::ScaleItem { corner, .. }) => corner.cursor(),
      _ if response.hovered() => pointer
        .and_then(|pointer| {
          self
            .selected_item
            .and_then(|index| resize_handle_at(&self.items[index], transform, pointer))
            .map(ResizeCorner::cursor)
            .or_else(|| {
              hit_test_items(&self.items, transform, pointer).map(|_| egui::CursorIcon::Move)
            })
        })
        .unwrap_or(egui::CursorIcon::Grab),
      _ => egui::CursorIcon::Default,
    };
    if response.hovered() || self.interaction.is_some() {
      ui.ctx().set_cursor_icon(cursor);
    }
  }

  fn draw_canvas(&mut self, ui: &mut egui::Ui) {
    let available = ui.available_size();
    let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, 0.0, Color32::from_rgb(239, 241, 244));

    if self.needs_layout {
      self.perform_layout(rect);
    }
    if self.needs_fit {
      self.fit_all(rect);
    }

    self.focus_requested_finding(rect);

    if !self.imports.is_busy() {
      self.handle_canvas_input(ui, &response, rect);
    }

    if self.items.is_empty() {
      draw_empty_state(&painter, rect);
    } else {
      let transform = self.transform(rect);
      let painter = painter.with_clip_rect(rect);
      for (index, item) in self.items.iter().enumerate() {
        draw_item(
          &painter,
          item,
          transform,
          rect,
          self.selected_item == Some(index) && self.measurements.tool == Tool::Select,
          self.label_font_size,
        );
      }
      if self.diagnostics.enabled {
        for (item, report) in self.items.iter().zip(&self.diagnostics.reports) {
          paint_report(&painter, item, report, transform, self.diagnostics.filter);
        }
      }
      for dimension in &self.measurements.completed {
        if let Some(item) = self.items.get(dimension.item) {
          paint_dimension(
            &painter,
            dimension,
            item,
            transform,
            self.label_font_size,
            false,
          );
        }
      }
      if let Some(snap) = self.measurements.start_snap()
        && let Some(item) = self.items.get(snap.item)
      {
        paint_snap(&painter, snap, item, transform);
      }
      if let Some(pointer) = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|pointer| rect.contains(*pointer))
      {
        if let Some(dimension) = self.measurements.preview(&self.items, transform, pointer) {
          paint_dimension(
            &painter,
            &dimension,
            &self.items[dimension.item],
            transform,
            self.label_font_size,
            true,
          );
        }
        if let Some(snap) = self
          .measurements
          .hover_snap(&self.items, transform, pointer)
        {
          paint_snap(&painter, snap, &self.items[snap.item], transform);
        }
        if let Some(pick) = self
          .measurements
          .hover_round(&self.items, transform, pointer)
        {
          paint_round_highlight(&painter, pick.curve, &self.items[pick.item], transform);
        }
      }
      if self.diagnostics.enabled
        && let Some(selection) = self.diagnostics.selected
        && let Some(item) = self.items.get(selection.item)
        && let Some(finding) = self
          .diagnostics
          .reports
          .get(selection.item)
          .and_then(|report| report.findings.get(selection.finding))
      {
        paint_selected_finding(&painter, item, finding, selection.finding, transform, rect);
      }
      draw_canvas_help(
        &painter,
        rect,
        self
          .measurements
          .notice
          .as_deref()
          .unwrap_or(self.measurements.hint()),
      );
    }

    if !ui.input(|input| input.raw.hovered_files.is_empty()) {
      painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(30, 92, 170, 38));
      painter.rect_stroke(
        rect.shrink(12.0),
        10.0,
        Stroke::new(2.0, Color32::from_rgb(30, 92, 170)),
        StrokeKind::Inside,
      );
      painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "Отпустите DXF-файлы здесь",
        FontId::proportional(24.0),
        Color32::from_rgb(24, 71, 132),
      );
    }
  }

  fn zoom_at(&mut self, pointer: Pos2, canvas_rect: Rect, factor: f32) {
    let old_zoom = self.zoom;
    let new_zoom = (old_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    if (new_zoom - old_zoom).abs() < f32::EPSILON {
      return;
    }

    let center = canvas_rect.center();
    let world_under_pointer = Point::new(
      self.view_center.x + ((pointer.x - center.x) / old_zoom) as f64,
      self.view_center.y - ((pointer.y - center.y) / old_zoom) as f64,
    );
    self.zoom = new_zoom;
    self.view_center = Point::new(
      world_under_pointer.x - ((pointer.x - center.x) / new_zoom) as f64,
      world_under_pointer.y + ((pointer.y - center.y) / new_zoom) as f64,
    );
  }

  fn remove_item(&mut self, index: usize) {
    self.measurements.remove_item(index);
    self.items.remove(index);
    self.diagnostics.refresh(&self.items);
    self.selected_item = match self.selected_item {
      Some(selected) if selected == index => None,
      Some(selected) if selected > index => Some(selected - 1),
      selected => selected,
    };
    self.interaction = None;
    self.needs_layout = true;
    self.needs_fit = true;
  }

  fn handle_shortcuts_and_drop(&mut self, context: &egui::Context) {
    let dropped: Vec<_> = context.input(|input| {
      input
        .raw
        .dropped_files
        .iter()
        .map(|file| file.path().to_path_buf())
        .collect()
    });
    if !dropped.is_empty() {
      self.add_paths(dropped);
    }
    if self.imports.is_busy() {
      return;
    }
    if !context.egui_wants_keyboard_input() {
      if context.input(|input| input.key_pressed(egui::Key::Escape)) {
        self.measurements.cancel();
        self.interaction = None;
      }
      if context.input_mut(|input| {
        input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, egui::Key::Z))
      }) {
        self.measurements.undo();
      }
      for (key, tool) in [
        (egui::Key::V, Tool::Select),
        (egui::Key::L, Tool::Linear),
        (egui::Key::D, Tool::Diameter),
        (egui::Key::R, Tool::Radius),
        (egui::Key::G, Tool::Angle),
        (egui::Key::A, Tool::Region),
      ] {
        if context.input(|input| input.modifiers == Modifiers::NONE && input.key_pressed(key)) {
          self.measurements.set_tool(tool);
          self.interaction = None;
        }
      }
    }
    let open = context.input_mut(|input| {
      input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, egui::Key::O))
    });
    if open {
      self.choose_files();
    }

    let fit = context.input_mut(|input| {
      input.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, egui::Key::Num0))
    });
    if fit {
      self.needs_fit = true;
    }
  }
}

impl eframe::App for DxfCanvasApp {
  fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    let context = root_ui.ctx().clone();
    self.handle_shortcuts_and_drop(&context);
    self.poll_imports(&context);
    if self.imports.is_busy() {
      root_ui.disable();
    }

    egui::Panel::top("toolbar")
      .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(10.0))
      .show(root_ui, |ui| {
        ui.horizontal_wrapped(|ui| {
          ui.heading(RichText::new("DXF Холст").size(20.0));
          ui.menu_button(
            RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
              .size(13.0)
              .weak(),
            |ui| {
              ui.set_max_width(460.0);
              egui::ScrollArea::vertical().max_height(450.0).show(ui, |ui| {
                  ui.label(include_str!("../THIRD_PARTY_NOTICES.md"));
                  ui.collapsing("Лицензия приложения", |ui| {
                    ui.label(include_str!("../LICENSE"));
                  });
                  ui.collapsing("ACadSharp / CSUtilities", |ui| {
                    ui.label(include_str!("../docs/licenses/ACADSHARP-LICENSE.txt"));
                  });
                  ui.collapsing(".NET NativeAOT", |ui| {
                    ui.label(include_str!("../docs/licenses/DOTNET-LICENSE.txt"));
                    ui.label(include_str!("../docs/licenses/DOTNET-NATIVE-NOTICES.txt"));
                  });
              });
            },
          );
          ui.separator();
          if ui.button("+ Добавить DWG/DXF").clicked() {
            self.choose_files();
          }
          if ui
            .add_enabled(!self.items.is_empty(), egui::Button::new("Вписать всё"))
            .clicked()
          {
            self.needs_fit = true;
          }
          if ui
            .add_enabled(!self.items.is_empty(), egui::Button::new("Переложить"))
            .clicked()
          {
            self.needs_layout = true;
            self.needs_fit = true;
          }
          if ui
            .add_enabled(!self.items.is_empty(), egui::Button::new("Очистить"))
            .clicked()
          {
            self.items.clear();
            self.world_bounds = None;
            self.selected_item = None;
            self.interaction = None;
            self.measurements.clear();
            self.diagnostics.clear();
          }
          if ui.add_enabled(!self.items.is_empty(), egui::Button::new("Проверка DXF").selected(self.diagnostics.enabled))
            .on_hover_text("Включить / выключить подсветку мест для проверки и легенду. Исходные DXF не меняются.")
            .clicked()
          {
            self.diagnostics.toggle(&self.items);
          }
          ui.separator();
          ui.add(
            egui::Slider::new(&mut self.label_font_size, 8.0..=36.0)
              .step_by(1.0)
              .text("Текст")
              .suffix(" px"),
          )
          .on_hover_text("Размер всех подписей на холсте");
          ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("Файлов: {}", self.items.len()));
          });
        });
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
          ui.label("Инструменты:");
          for (tool, label) in [
            (Tool::Select, "Выбор · V"),
            (Tool::Linear, "Линейный · L"),
            (Tool::Diameter, "Ø Диаметр · D"),
            (Tool::Radius, "R Радиус · R"),
            (Tool::Angle, "Угол · G"),
            (Tool::Region, "Площадь / периметр · A"),
          ] {
            if ui
              .selectable_label(self.measurements.tool == tool, label)
              .clicked()
            {
              self.measurements.set_tool(tool);
              self.interaction = None;
            }
          }
          ui.separator();
          if ui
            .button("Отменить размер")
            .on_hover_text("Ctrl+Z — убрать последний размер или отменить текущий")
            .clicked()
          {
            self.measurements.undo();
          }
          if ui
            .add_enabled(
              !self.measurements.completed.is_empty(),
              egui::Button::new("Убрать размеры"),
            )
            .clicked()
          {
            self.measurements.clear();
          }
          ui.label(format!("Размеров: {}", self.measurements.completed.len()));
        });
      });

    if !self.items.is_empty() {
      egui::Panel::right("files")
        .default_size(290.0)
        .min_size(220.0)
        .max_size(380.0)
        .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(12.0))
        .show(root_ui, |ui| {
          ui.heading("Файлы на холсте");
          ui.label(
            RichText::new("Нажмите на имя, чтобы выбрать деталь")
              .small()
              .color(Color32::from_gray(105)),
          );
          ui.add_space(6.0);
          let mut remove = None;
          let mut selected = self.selected_item;
          let mut item_changed = false;
          let mut clicked_finding = None;
          let mut layers_changed = false;
          let single_file = self.items.len() == 1;
          egui::ScrollArea::vertical().show(ui, |ui| {
            if self.diagnostics.enabled {
              show_legend(ui, &mut self.diagnostics);
            }
            for (index, item) in self.items.iter_mut().enumerate() {
              let is_selected = selected == Some(index);
              egui::Frame::new()
                .fill(if is_selected {
                  Color32::from_rgb(236, 244, 255)
                } else {
                  Color32::from_rgb(246, 247, 249)
                })
                .stroke(if is_selected {
                  Stroke::new(1.2, Color32::from_rgb(37, 105, 193))
                } else {
                  Stroke::NONE
                })
                .corner_radius(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                  ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                      if ui
                        .selectable_label(is_selected, RichText::new(&item.name).strong())
                        .clicked()
                      {
                        selected = Some(index);
                      }
                      ui.label(
                        RichText::new(format!(
                          "{:.1} × {:.1} {} · {} элементов",
                          item.bounds.width() * item.units.factor(),
                          item.bounds.height() * item.units.factor(),
                          item.units.label(),
                          item.primitives.len()
                            + item.appearance.texts.len()
                            + item.appearance.fills.len()
                        ))
                        .small()
                        .color(Color32::from_gray(95)),
                      );
                      if item.unsupported_entities > 0 {
                        ui.label(
                          RichText::new(format!(
                            "Не показано сущностей: {}",
                            item.unsupported_entities
                          ))
                          .small()
                          .color(Color32::from_rgb(166, 99, 20)),
                        );
                      }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                      if ui.small_button("×").on_hover_text("Убрать файл").clicked() {
                        remove = Some(index);
                      }
                    });
                  });

                  if self.diagnostics.enabled
                    && let Some(report) = self.diagnostics.reports.get(index)
                  {
                    let selected_finding = self
                      .diagnostics
                      .selected
                      .filter(|selection| selection.item == index)
                      .map(|selection| selection.finding);
                    if let Some(finding) =
                      show_file_report(ui, report, index, selected_finding, self.diagnostics.filter)
                    {
                      clicked_finding = Some((index, finding));
                      selected = Some(index);
                    }
                  }
                  if is_selected || single_file {
                    layers_changed |=
                      crate::cad_render::layers_ui(ui, item, &mut self.layer_filter);
                    if !item.appearance.warnings.is_empty() {
                      ui.collapsing("Предупреждения импорта", |ui| {
                        for warning in &item.appearance.warnings {
                          ui.colored_label(Color32::from_rgb(160, 90, 20), warning);
                        }
                      });
                    }
                  }
                  if is_selected {
                    ui.separator();
                    ui.horizontal(|ui| {
                      ui.label(format!("Размер: {:.0}%", item.scale * 100.0));
                      if ui
                        .add(
                          egui::Slider::new(&mut item.scale, MIN_ITEM_SCALE..=MAX_ITEM_SCALE)
                            .logarithmic(true)
                            .show_value(false),
                        )
                        .on_hover_text("Индивидуальный масштаб детали")
                        .changed()
                      {
                        item_changed = true;
                      }
                    });
                    ui.horizontal(|ui| {
                      if ui.small_button("-").clicked() {
                        item.scale = (item.scale / 1.1).max(MIN_ITEM_SCALE);
                        item_changed = true;
                      }
                      if ui.small_button("100%").clicked() {
                        item.scale = 1.0;
                        item_changed = true;
                      }
                      if ui.small_button("+").clicked() {
                        item.scale = (item.scale * 1.1).min(MAX_ITEM_SCALE);
                        item_changed = true;
                      }
                      ui.label(
                        RichText::new("Ctrl+колесо над деталью")
                          .small()
                          .color(Color32::from_gray(100)),
                      );
                    });
                  }
                });
              ui.add_space(6.0);
            }
          });
          self.selected_item = selected;
          if layers_changed {
            self.diagnostics.refresh(&self.items);
            self.measurements.cancel();
          }
          if let Some((item, finding)) = clicked_finding {
            self.diagnostics.select(item, finding);
          }
          if item_changed {
            self.recalculate_world_bounds();
          }
          if let Some(index) = remove {
            self.remove_item(index);
          }
        });
    }

    if !self.errors.is_empty() {
      egui::Panel::bottom("errors")
        .resizable(true)
        .default_size(42.0)
        .show(root_ui, |ui| {
          ui.horizontal(|ui| {
            ui.colored_label(
              Color32::from_rgb(175, 45, 45),
              "Некоторые файлы не открыты:",
            );
            if ui.small_button("Скрыть").clicked() {
              self.errors.clear();
            }
          });
          for error in &self.errors {
            ui.label(error);
          }
        });
    }

    egui::CentralPanel::default()
      .frame(egui::Frame::NONE)
      .show(root_ui, |ui| self.draw_canvas(ui));
    show_loading(&context, &self.imports);
  }
}

fn draw_item(
  painter: &egui::Painter,
  item: &DrawingItem,
  transform: ViewTransform,
  canvas_rect: Rect,
  selected: bool,
  font_size: f32,
) {
  crate::cad_render::paint(painter, item, transform);

  let placed = item.placed_bounds();
  let screen_left = transform.world_to_screen(Point::new(placed.min.x, placed.max.y));
  let galley = painter.layout(
    item.name.clone(),
    FontId::new(font_size, FontFamily::Proportional),
    Color32::from_rgb(24, 28, 34),
    (canvas_rect.width() - 22.0).max(1.0),
  );
  let min_label_x = canvas_rect.left() + 6.0;
  let max_label_x = (canvas_rect.right() - galley.size().x - 6.0).max(min_label_x);
  let label_x = screen_left.x.clamp(min_label_x, max_label_x);
  let label_pos = egui::pos2(label_x, screen_left.y - galley.size().y - 9.0);
  let background = Rect::from_min_size(label_pos, galley.size()).expand2(Vec2::new(5.0, 3.0));
  painter.rect_filled(
    background,
    3.0,
    Color32::from_rgba_unmultiplied(255, 255, 255, 235),
  );
  painter.galley(label_pos, galley, Color32::from_rgb(24, 28, 34));

  if selected {
    let selection_rect = item_screen_rect(item, transform).expand(5.0);
    painter.rect_stroke(
      selection_rect,
      1.0,
      Stroke::new(1.3, Color32::from_rgb(37, 105, 193)),
      StrokeKind::Inside,
    );
    for (_, position) in resize_handles(selection_rect) {
      let handle = Rect::from_center_size(position, Vec2::splat(RESIZE_HANDLE_SIZE));
      painter.rect_filled(handle, 1.0, Color32::WHITE);
      painter.rect_stroke(
        handle,
        1.0,
        Stroke::new(1.5, Color32::from_rgb(37, 105, 193)),
        StrokeKind::Inside,
      );
    }
  }
}

fn item_screen_rect(item: &DrawingItem, transform: ViewTransform) -> Rect {
  let bounds = item.placed_bounds();
  Rect::from_two_pos(
    transform.world_to_screen(Point::new(bounds.min.x, bounds.max.y)),
    transform.world_to_screen(Point::new(bounds.max.x, bounds.min.y)),
  )
}

fn resize_handles(rect: Rect) -> [(ResizeCorner, Pos2); 4] {
  [
    (ResizeCorner::TopLeft, rect.left_top()),
    (ResizeCorner::TopRight, rect.right_top()),
    (ResizeCorner::BottomLeft, rect.left_bottom()),
    (ResizeCorner::BottomRight, rect.right_bottom()),
  ]
}

fn resize_handle_at(
  item: &DrawingItem,
  transform: ViewTransform,
  pointer: Pos2,
) -> Option<ResizeCorner> {
  resize_handles(item_screen_rect(item, transform).expand(5.0))
    .into_iter()
    .find(|(_, position)| {
      Rect::from_center_size(*position, Vec2::splat(RESIZE_HANDLE_SIZE + 7.0)).contains(pointer)
    })
    .map(|(corner, _)| corner)
}

fn hit_test_items(items: &[DrawingItem], transform: ViewTransform, pointer: Pos2) -> Option<usize> {
  items
    .iter()
    .enumerate()
    .rev()
    .find(|(_, item)| {
      item_screen_rect(item, transform)
        .expand(5.0)
        .contains(pointer)
    })
    .map(|(index, _)| index)
}

fn distance(left: Point, right: Point) -> f64 {
  (left.x - right.x).hypot(left.y - right.y)
}

fn move_item_by_screen_delta(item: &mut DrawingItem, delta: Vec2, view_scale: f32) {
  item.offset.x += delta.x as f64 / view_scale as f64;
  item.offset.y -= delta.y as f64 / view_scale as f64;
}

fn draw_canvas_help(painter: &egui::Painter, rect: Rect, text: &str) {
  let galley = painter.layout_no_wrap(
    text.to_owned(),
    FontId::proportional(12.0),
    Color32::from_rgb(88, 98, 110),
  );
  let position = egui::pos2(
    rect.center().x - galley.size().x * 0.5,
    rect.bottom() - galley.size().y - 12.0,
  );
  let background = Rect::from_min_size(position, galley.size()).expand2(Vec2::new(7.0, 4.0));
  painter.rect_filled(
    background,
    4.0,
    Color32::from_rgba_unmultiplied(255, 255, 255, 222),
  );
  painter.galley(position, galley, Color32::from_rgb(88, 98, 110));
}

fn draw_empty_state(painter: &egui::Painter, rect: Rect) {
  let center = rect.center();
  painter.text(
    center - Vec2::new(0.0, 32.0),
    Align2::CENTER_CENTER,
    "Перетащите сюда DWG- или DXF-файлы",
    FontId::proportional(25.0),
    Color32::from_rgb(58, 67, 79),
  );
  painter.text(
    center + Vec2::new(0.0, 3.0),
    Align2::CENTER_CENTER,
    "или нажмите «Добавить DWG/DXF»",
    FontId::proportional(16.0),
    Color32::from_rgb(111, 120, 132),
  );
  painter.text(
    center + Vec2::new(0.0, 36.0),
    Align2::CENTER_CENTER,
    "Колесо — масштаб · перетаскивание — перемещение",
    FontId::proportional(13.0),
    Color32::from_rgb(135, 143, 153),
  );
}

fn configure_fonts_and_style(context: &egui::Context) {
  let mut fonts = egui::FontDefinitions::default();
  let candidates = [
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
  ];
  if let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) {
    fonts.font_data.insert(
      "system-cyrillic".to_owned(),
      Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
      fonts
        .families
        .entry(family)
        .or_default()
        .insert(0, "system-cyrillic".to_owned());
    }
  }
  context.set_fonts(fonts);

  let mut style = (*context.style_of(egui::Theme::Light)).clone();
  style.spacing.button_padding = Vec2::new(10.0, 6.0);
  style.visuals = egui::Visuals::light();
  style.visuals.widgets.active.bg_fill = Color32::from_rgb(35, 100, 181);
  style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(224, 233, 245);
  context.set_style_of(egui::Theme::Light, style);
  context.set_theme(egui::ThemePreference::Light);
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geometry::Primitive;

  fn item(name: &str, offset: Point) -> DrawingItem {
    DrawingItem {
      appearance: Default::default(),
      units: Default::default(),
      path: PathBuf::from(name),
      name: name.to_owned(),
      primitives: vec![],
      bounds: Bounds {
        min: Point::new(0.0, 0.0),
        max: Point::new(100.0, 50.0),
      },
      offset,
      scale: 1.0,
      unsupported_entities: 0,
    }
  }

  fn focus_test_app() -> DxfCanvasApp {
    let mut target = item("target.dxf", Point::new(300.0, 200.0));
    target.scale = 2.0;
    target.units = crate::geometry::LengthUnit::from_dxf_code(4);
    target.primitives = vec![Primitive::Path {
      points: vec![Point::new(20.0, 30.0), Point::new(30.0, 30.0)],
      closed: false,
      curves: vec![crate::geometry::MeasureCurve::Line {
        start: Point::new(20.0, 30.0),
        end: Point::new(30.0, 30.0),
      }],
    }];
    let mut app = DxfCanvasApp {
      imports: ImportQueue::default(),
      layer_filter: String::new(),
      items: vec![item("other.dxf", Point::default()), target],
      errors: vec![],
      world_bounds: None,
      view_center: Point::default(),
      zoom: 0.1,
      needs_layout: false,
      needs_fit: false,
      selected_item: Some(0),
      interaction: None,
      label_font_size: 20.0,
      measurements: MeasurementState::default(),
      diagnostics: DiagnosticsState::default(),
    };
    app.measurements.set_tool(Tool::Linear);
    app
      .measurements
      .completed
      .push(crate::measurement::Dimension::linear(
        1,
        Point::new(20.0, 30.0),
        Point::new(30.0, 30.0),
        Point::new(25.0, 40.0),
      ));
    app.diagnostics.toggle(&app.items);
    app
  }

  #[test]
  fn background_import_continues_after_error_and_keeps_existing_measurements() {
    let context = egui::Context::default();
    let mut app = focus_test_app();
    let before = format!("{:?}", app.measurements.completed);
    let path = std::env::temp_dir().join(format!("dxf-canvas-loading-{}.dxf", std::process::id()));
    crate::test_fixtures::diagnostics_drawing()
      .save_file(&path)
      .unwrap();
    let missing = path.with_file_name(format!(
      "missing-dxf-canvas-loading-{}.dxf",
      std::process::id()
    ));
    app.add_paths(vec![missing, path.clone(), path.clone()]);
    assert!(app.imports.is_busy());
    assert_eq!(app.items.len(), 2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.imports.is_busy() {
      app.poll_imports(&context);
      assert!(
        std::time::Instant::now() < deadline,
        "Фоновый импорт не завершился"
      );
      std::thread::sleep(std::time::Duration::from_millis(1));
    }
    std::fs::remove_file(path).unwrap();
    assert_eq!(app.errors.len(), 1);
    assert_eq!(app.items.len(), 3);
    assert_eq!(app.diagnostics.reports.len(), 3);
    assert_eq!(format!("{:?}", app.measurements.completed), before);
    assert!(app.needs_layout && app.needs_fit);
    assert!(app.imports.progress().is_none());
  }

  #[test]
  fn focusing_a_finding_changes_only_the_view_and_selection() {
    let mut app = focus_test_app();
    let geometry_before = format!("{:?}", app.items);
    let dimensions_before = format!("{:?}", app.measurements.completed);
    assert!(app.diagnostics.select(1, 0));
    app.focus_requested_finding(Rect::from_min_size(
      Pos2::new(0.0, 120.0),
      Vec2::new(1000.0, 800.0),
    ));
    assert_eq!(app.selected_item, Some(1));
    assert_eq!(app.view_center, Point::new(290.0, 235.0));
    assert!((app.zoom - 130.0).abs() < 1.0e-4);
    assert_eq!(format!("{:?}", app.items), geometry_before);
    assert_eq!(
      format!("{:?}", app.measurements.completed),
      dimensions_before
    );
    assert_eq!(app.measurements.tool, Tool::Linear);
    assert_eq!(app.label_font_size, 20.0);
    app.view_center = Point::new(10.0, 10.0);
    app.focus_requested_finding(Rect::EVERYTHING);
    assert_eq!(app.view_center, Point::new(10.0, 10.0));
  }

  #[test]
  fn removing_a_file_cancels_a_pending_focus() {
    let mut app = focus_test_app();
    assert!(app.diagnostics.select(1, 0));
    app.remove_item(0);
    assert!(app.diagnostics.selected.is_none());
    assert!(app.diagnostics.take_focus_request().is_none());
  }

  #[test]
  fn moving_one_item_does_not_change_another() {
    let mut items = [
      item("first.dxf", Point::new(0.0, 0.0)),
      item("second.dxf", Point::new(300.0, 200.0)),
    ];
    let second_before = items[1].placed_bounds();

    move_item_by_screen_delta(&mut items[0], Vec2::new(40.0, -20.0), 2.0);

    assert_eq!(items[0].offset, Point::new(20.0, 10.0));
    assert_eq!(items[1].placed_bounds(), second_before);
  }

  #[test]
  fn scaling_keeps_opposite_corner_fixed() {
    let mut item = item("detail.dxf", Point::new(20.0, 30.0));
    let anchor_local = Point::new(item.bounds.max.x, item.bounds.min.y);
    let anchor_world = item.world_point(anchor_local);

    item.set_scale_keeping_anchor(2.5, anchor_local, anchor_world);

    assert_eq!(item.world_point(anchor_local), anchor_world);
    assert_eq!(item.scale, 2.5);
  }

  #[test]
  fn all_labels_use_the_requested_size_regardless_of_item_and_view_scale() {
    let context = egui::Context::default();
    for font_size in [8.0, 16.0, 36.0] {
      for view_scale in [0.25, 2.0] {
        let mut output = context.run_ui(Default::default(), |ui| {
          for item_scale in [0.1, 5.0] {
            let mut drawing = item(&"длинное_название_".repeat(20), Point::default());
            drawing.scale = item_scale;
            draw_item(
              ui.painter(),
              &drawing,
              ViewTransform {
                scale: view_scale,
                origin: egui::pos2(100.0, 400.0),
              },
              Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)),
              false,
              font_size,
            );
          }
        });
        output.textures_delta.clear();
        let sizes: Vec<_> = output
          .shapes
          .iter()
          .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
              Some(text.galley.job.sections[0].format.font_id.size)
            } else {
              None
            }
          })
          .collect();
        assert_eq!(sizes, vec![font_size, font_size]);
      }
    }
  }
}
