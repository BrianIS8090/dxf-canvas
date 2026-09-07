use std::{path::PathBuf, sync::Arc};

use eframe::egui::{
  self, Align2, Color32, FontFamily, FontId, KeyboardShortcut, Modifiers, PointerButton, Pos2,
  Rect, RichText, Sense, Stroke, StrokeKind, Vec2,
};

use crate::{
  diagnostics::DiagnosticsState,
  diagnostics_ui::{paint_report, paint_selected_finding, show_file_report, show_legend},
  geometry::{Bounds, DrawingItem, Point, ViewTransform},
  layout::{arrange, place_new_item},
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
const ROTATION_HANDLE_RADIUS: f32 = 10.0;
const DEFAULT_LABEL_FONT_SIZE: f32 = 16.0;

mod workspace_ui;

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
  RotateItem {
    index: usize,
    start_angle: f64,
    start_rotation: f64,
  },
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
  fit_initial_import: bool,
  selected_item: Option<usize>,
  interaction: Option<CanvasInteraction>,
  label_font_size: f32,
  measurements: MeasurementState,
  diagnostics: DiagnosticsState,
  layer_filter: String,
  imports: ImportQueue,
  checking: crate::checking::CheckJob,
  workspace: workspace_ui::WorkspaceUi,
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
      fit_initial_import: false,
      selected_item: None,
      interaction: None,
      label_font_size: DEFAULT_LABEL_FONT_SIZE,
      measurements: MeasurementState::default(),
      diagnostics: DiagnosticsState::default(),
      layer_filter: String::new(),
      imports: ImportQueue::default(),
      checking: crate::checking::CheckJob::default(),
      workspace: workspace_ui::WorkspaceUi::default(),
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
      if self.items.is_empty() {
        self.fit_initial_import = true;
      }
      self.interaction = None;
    }
  }

  fn poll_imports(&mut self, context: &egui::Context) {
    if let Some(result) = self.imports.poll(context, self.diagnostics.enabled) {
      match result {
        Ok(mut loaded) => {
          if let Some(report) = loaded.report {
            self.diagnostics.reports.push(report);
          }
          self.diagnostics.clear_selection();
          place_new_item(&mut loaded.item, &self.items);
          self.items.push(loaded.item);
          self.selected_item = Some(self.items.len() - 1);
          self.recalculate_world_bounds();
        }
        Err(error) => self.errors.push(error),
      }
    }
    // Вписываем только первую загрузку в пустое окно. Последующие импорты не двигают вид.
    if self.fit_initial_import && !self.imports.is_busy() {
      self.needs_fit = !self.items.is_empty();
      self.fit_initial_import = false;
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
      shift,
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
        input.modifiers.shift,
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
      && let Some(pointer) = ui.input(|input| input.pointer.press_origin()).or(pointer)
    {
      let handle = self.selected_item.and_then(|index| {
        resize_handle_at(&self.items[index], transform, pointer).map(|corner| (index, corner))
      });
      let rotate = self
        .selected_item
        .filter(|&index| rotation_handle_at(&self.items[index], transform, pointer));
      if let Some(index) = rotate {
        let item = &self.items[index];
        let center = transform.world_to_screen(item.world_point(item.bounds.center()));
        self.interaction = Some(CanvasInteraction::RotateItem {
          index,
          start_angle: pointer_angle(center, pointer),
          start_rotation: item.rotation.radians(),
        });
      } else if let Some((index, corner)) = handle {
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
    // Shift должен менять шаг даже при неподвижной мыши во время перетаскивания.
    if (primary_down || primary_released)
      && let Some(CanvasInteraction::RotateItem {
        index,
        start_angle,
        start_rotation,
      }) = self.interaction
      && let (Some(item), Some(pointer)) = (self.items.get_mut(index), pointer)
    {
      let center = transform.world_to_screen(item.world_point(item.bounds.center()));
      if center.distance(pointer) > 3.0 {
        item.rotation = crate::geometry::Rotation::new(rotation_angle(
          start_rotation,
          start_angle,
          pointer_angle(center, pointer),
          shift,
        ));
        item_changed = true;
      }
    }
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

    if response.hovered()
      && scroll.abs() > 0.01
      && !matches!(self.interaction, Some(CanvasInteraction::RotateItem { .. }))
    {
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
      Some(CanvasInteraction::RotateItem { .. }) => egui::CursorIcon::Grabbing,
      Some(CanvasInteraction::ScaleItem { index, corner, .. }) => {
        resize_cursor(&self.items[index], corner)
      }
      _ if response.hovered() => pointer
        .and_then(|pointer| {
          self
            .selected_item
            .and_then(|index| {
              let item = &self.items[index];
              if rotation_handle_at(item, transform, pointer) {
                Some(egui::CursorIcon::Grab)
              } else {
                resize_handle_at(item, transform, pointer).map(|corner| resize_cursor(item, corner))
              }
            })
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
    if response.hovered()
      && pointer.is_some_and(|p| {
        self
          .selected_item
          .is_some_and(|i| rotation_handle_at(&self.items[i], transform, p))
      })
    {
      response
        .clone()
        .on_hover_text("Потяните круглый маркер — поворот вокруг центра. Shift — шаг 45°.");
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
    self.focus_workspace_selection(rect);

    if !self.imports.is_busy() {
      self.handle_canvas_input(ui, &response, rect);
    }

    if self.items.is_empty() {
      draw_empty_state(&painter, rect);
      let button_rect =
        Rect::from_center_size(rect.center() + Vec2::new(0.0, 90.0), Vec2::new(230.0, 38.0));
      if ui
        .put(button_rect, egui::Button::new("Открыть файлы · Ctrl+O"))
        .clicked()
      {
        self.choose_files();
      }
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
        "Отпустите DWG- или DXF-файлы здесь",
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
    if index < self.diagnostics.reports.len() {
      self.diagnostics.reports.remove(index);
    }
    self.diagnostics.clear_selection();
    self.selected_item = match self.selected_item {
      Some(selected) if selected == index => None,
      Some(selected) if selected > index => Some(selected - 1),
      selected => selected,
    };
    self.interaction = None;
    self.recalculate_world_bounds();
    if self.items.is_empty() {
      self.needs_layout = false;
      self.needs_fit = false;
    }
  }

  fn handle_shortcuts_and_drop(&mut self, context: &egui::Context) {
    if self.checking.is_busy() || self.workspace_dialog_open() {
      return;
    }
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
      if context.input(|input| input.key_pressed(egui::Key::F1)) {
        self.workspace.help_open = !self.workspace.help_open;
      }
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
    if !self.checking.is_busy() {
      self.poll_imports(&context);
    }
    if let Some(result) = self.checking.poll(&context, &self.items) {
      match result {
        Ok(reports) => self.diagnostics.reports = reports,
        Err(error) => {
          self.diagnostics.clear();
          self.errors.push(error);
        }
      }
    }
    if self.imports.is_busy() || self.checking.is_busy() {
      root_ui.disable();
    }

    self.show_toolbar(root_ui);
    self.show_inspector(root_ui);
    self.show_status(root_ui);

    if !self.errors.is_empty() {
      egui::Panel::bottom("errors")
        .resizable(true)
        .default_size(42.0)
        .show(root_ui, |ui| {
          ui.horizontal(|ui| {
            ui.colored_label(Color32::from_rgb(175, 45, 45), "Сообщения об ошибках:");
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
    self.checking.show(&context);
    self.show_workspace_dialogs(&context);
  }
}

fn region_mode_controls(ui: &mut egui::Ui, measurements: &mut MeasurementState) {
  ui.horizontal_wrapped(|ui| {
    ui.label("Считать:");
    let mut contour_only = measurements.contour_only;
    ui.selectable_value(&mut contour_only, false, "Деталь с отверстиями");
    ui.selectable_value(&mut contour_only, true, "Отдельный контур")
      .on_hover_text(
        "Для отдельных элементов на большом плане. Вложенные отверстия не вычитаются.",
      );
    ui.label("S — м² · P — м");
    if contour_only != measurements.contour_only {
      measurements.set_tool(Tool::Region);
      measurements.contour_only = contour_only;
    }
  });
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

  let screen_left = item_screen_rect(item, transform).left_top();
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
    let handles = selection_handles(item, transform);
    painter.add(egui::Shape::closed_line(
      vec![handles[0].1, handles[1].1, handles[3].1, handles[2].1],
      Stroke::new(1.3, Color32::from_rgb(37, 105, 193)),
    ));
    for (_, position) in handles {
      let handle = Rect::from_center_size(position, Vec2::splat(RESIZE_HANDLE_SIZE));
      painter.rect_filled(handle, 1.0, Color32::WHITE);
      painter.rect_stroke(
        handle,
        1.0,
        Stroke::new(1.5, Color32::from_rgb(37, 105, 193)),
        StrokeKind::Inside,
      );
    }
    let (anchor, knob) = rotation_handle(item, transform);
    let color = Color32::from_rgb(37, 105, 193);
    painter.line_segment([anchor, knob], Stroke::new(1.3, color));
    painter.circle(
      knob,
      ROTATION_HANDLE_RADIUS,
      Color32::WHITE,
      Stroke::new(1.8, color),
    );
    painter.text(
      knob,
      Align2::CENTER_CENTER,
      "↻",
      FontId::proportional(17.0),
      color,
    );
    let angle = item.rotation.radians().to_degrees().rem_euclid(360.0);
    let text = format!("{angle:.1}°");
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), color);
    let min_x = canvas_rect.left() + 5.0;
    let max_x = (canvas_rect.right() - galley.size().x - 5.0).max(min_x);
    let pos = egui::pos2(
      (knob.x - galley.size().x * 0.5).clamp(min_x, max_x),
      knob.y + 16.0,
    );
    painter.rect_filled(
      Rect::from_min_size(pos, galley.size()).expand(3.0),
      3.0,
      Color32::WHITE,
    );
    painter.galley(pos, galley, color);
  }
}

fn item_screen_rect(item: &DrawingItem, transform: ViewTransform) -> Rect {
  let bounds = item.placed_bounds();
  Rect::from_two_pos(
    transform.world_to_screen(Point::new(bounds.min.x, bounds.max.y)),
    transform.world_to_screen(Point::new(bounds.max.x, bounds.min.y)),
  )
}

fn selection_handles(item: &DrawingItem, transform: ViewTransform) -> [(ResizeCorner, Pos2); 4] {
  let padding = 5.0 / (item.scale * transform.scale as f64).max(1.0e-12);
  let min = Point::new(item.bounds.min.x - padding, item.bounds.min.y - padding);
  let max = Point::new(item.bounds.max.x + padding, item.bounds.max.y + padding);
  [
    (ResizeCorner::TopLeft, Point::new(min.x, max.y)),
    (ResizeCorner::TopRight, max),
    (ResizeCorner::BottomLeft, min),
    (ResizeCorner::BottomRight, Point::new(max.x, min.y)),
  ]
  .map(|(corner, point)| (corner, transform.world_to_screen(item.world_point(point))))
}

fn rotation_handle(item: &DrawingItem, transform: ViewTransform) -> (Pos2, Pos2) {
  let handles = selection_handles(item, transform);
  let anchor = handles[1].1.lerp(handles[3].1, 0.5);
  let direction = handles[1].1 - handles[0].1;
  (anchor, anchor + direction.normalized() * 32.0)
}

fn rotation_handle_at(item: &DrawingItem, transform: ViewTransform, pointer: Pos2) -> bool {
  rotation_handle(item, transform).1.distance(pointer) <= ROTATION_HANDLE_RADIUS + 5.0
}

fn pointer_angle(center: Pos2, pointer: Pos2) -> f64 {
  (center.y as f64 - pointer.y as f64).atan2(pointer.x as f64 - center.x as f64)
}

fn rotation_angle(start_rotation: f64, start_angle: f64, current_angle: f64, snap: bool) -> f64 {
  let delta = current_angle - start_angle;
  let angle = start_rotation + delta.sin().atan2(delta.cos());
  if snap {
    let step = std::f64::consts::FRAC_PI_4;
    (angle / step).round() * step
  } else {
    angle
  }
}

fn resize_cursor(item: &DrawingItem, corner: ResizeCorner) -> egui::CursorIcon {
  if item.rotation.radians().abs() < 1.0e-9 {
    return corner.cursor();
  }
  let local = corner.opposite_local(item.bounds);
  let center = item.bounds.center();
  let vector = item
    .rotation
    .apply(Point::new(local.x - center.x, local.y - center.y));
  let sector = ((-vector.y).atan2(vector.x) / std::f64::consts::FRAC_PI_4).round() as i32;
  match sector.rem_euclid(4) {
    0 => egui::CursorIcon::ResizeHorizontal,
    1 => egui::CursorIcon::ResizeNwSe,
    2 => egui::CursorIcon::ResizeVertical,
    _ => egui::CursorIcon::ResizeNeSw,
  }
}

fn resize_handle_at(
  item: &DrawingItem,
  transform: ViewTransform,
  pointer: Pos2,
) -> Option<ResizeCorner> {
  selection_handles(item, transform)
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
      let local = item.local_point(transform.screen_to_world(pointer));
      let pad = 5.0 / (item.scale * transform.scale as f64).max(1.0e-12);
      local.x >= item.bounds.min.x - pad
        && local.x <= item.bounds.max.x + pad
        && local.y >= item.bounds.min.y - pad
        && local.y <= item.bounds.max.y + pad
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
    "Чертежи, слои, измерения и проверка геометрии",
    FontId::proportional(16.0),
    Color32::from_rgb(111, 120, 132),
  );
  painter.text(
    center + Vec2::new(0.0, 36.0),
    Align2::CENTER_CENTER,
    "Исходные файлы остаются без изменений",
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
  style
    .text_styles
    .insert(egui::TextStyle::Body, FontId::proportional(14.0));
  style
    .text_styles
    .insert(egui::TextStyle::Button, FontId::proportional(14.0));
  style
    .text_styles
    .insert(egui::TextStyle::Small, FontId::proportional(12.0));
  style
    .text_styles
    .insert(egui::TextStyle::Heading, FontId::proportional(18.0));
  style.spacing.button_padding = Vec2::new(10.0, 6.0);
  style.spacing.item_spacing = Vec2::new(6.0, 6.0);
  style.visuals = egui::Visuals::light();
  style.visuals.override_text_color = Some(Color32::from_rgb(37, 48, 63));
  style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(245, 247, 250);
  style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(222, 228, 235));
  style.visuals.selection.bg_fill = Color32::from_rgb(220, 235, 252);
  style.visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(23, 75, 135));
  style.visuals.widgets.active.bg_fill = Color32::from_rgb(210, 229, 249);
  style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(224, 233, 245);
  context.set_style_of(egui::Theme::Light, style);
  context.set_theme(egui::ThemePreference::Light);
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geometry::Primitive;

  #[test]
  fn clicking_area_modes_keeps_the_area_tool_active() {
    let context = egui::Context::default();
    let mut measurements = MeasurementState::default();
    measurements.set_tool(Tool::Region);
    for (label, expected) in [("Отдельный контур", true), ("Деталь с отверстиями", false)]
    {
      let mut frame = |events| {
        let mut output = context.run_ui(
          egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 150.0))),
            events,
            ..Default::default()
          },
          |ui| region_mode_controls(ui, &mut measurements),
        );
        output.textures_delta.clear();
        output
      };
      let output = frame(vec![]);
      let target = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
          egui::Shape::Text(text) if text.galley.job.text == label => {
            Some(text.pos + text.galley.size() * 0.5)
          }
          _ => None,
        })
        .expect("Нет кнопки режима площади");
      for pressed in [true, false] {
        frame(vec![
          egui::Event::PointerMoved(target),
          egui::Event::PointerButton {
            pos: target,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
          },
        ]);
      }
      assert_eq!(measurements.contour_only, expected);
      assert_eq!(
        measurements.tool,
        Tool::Region,
        "Кнопка «{label}» выключила измерение площади"
      );
    }
  }

  fn item(name: &str, offset: Point) -> DrawingItem {
    DrawingItem {
      rotation: Default::default(),
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

  pub(super) fn focus_test_app() -> DxfCanvasApp {
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
      workspace: workspace_ui::WorkspaceUi::default(),
      imports: ImportQueue::default(),
      checking: crate::checking::CheckJob::default(),
      layer_filter: String::new(),
      items: vec![item("other.dxf", Point::default()), target],
      errors: vec![],
      world_bounds: None,
      view_center: Point::default(),
      zoom: 0.1,
      needs_layout: false,
      needs_fit: false,
      fit_initial_import: false,
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
  fn rotating_item_preserves_center_inverse_coordinates_and_scale_anchor() {
    let mut item = item("rotation.dxf", Point::new(3000.0, -1000.0));
    item.scale = 2.3;
    let center = item.world_point(item.bounds.center());
    for angle in [0.0_f64, 45.0, 90.0, 180.0, 271.25] {
      item.rotation = crate::geometry::Rotation::new(angle.to_radians());
      assert_eq!(item.world_point(item.bounds.center()), center);
      let bounds = item.placed_bounds();
      for point in item.bounds.corners() {
        let world = item.world_point(point);
        assert!(distance(item.local_point(world), point) < 1.0e-9);
        assert!(world.x >= bounds.min.x - 1.0e-9 && world.x <= bounds.max.x + 1.0e-9);
        assert!(world.y >= bounds.min.y - 1.0e-9 && world.y <= bounds.max.y + 1.0e-9);
      }
    }
    let anchor = item.bounds.min;
    let world = item.world_point(anchor);
    item.set_scale_keeping_anchor(4.2, anchor, world);
    assert!(distance(item.world_point(anchor), world) < 1.0e-9);
  }

  #[test]
  fn rotation_handle_drag_and_stationary_shift_snap_to_absolute_45_degrees() {
    let context = egui::Context::default();
    let mut app = focus_test_app();
    app.measurements.set_tool(Tool::Select);
    app.diagnostics.clear();
    app.zoom = 3.0;
    app.view_center = app.items[0].bounds.center();
    let source = format!("{:?}", app.items[0].primitives);
    let second = app.items[1].placed_bounds();
    let frame = |app: &mut DxfCanvasApp, mut events: Vec<egui::Event>, modifiers| {
      events.insert(0, egui::Event::ModifiersChanged(modifiers));
      let mut canvas = Rect::NOTHING;
      let mut output = context.run_ui(
        egui::RawInput {
          screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
          events,
          ..Default::default()
        },
        |ui| {
          canvas = ui.available_rect_before_wrap();
          app.draw_canvas(ui);
        },
      );
      output.textures_delta.clear();
      canvas
    };
    let canvas = frame(&mut app, vec![], Modifiers::NONE);
    let view = app.transform(canvas);
    let center = view.world_to_screen(app.items[0].world_point(app.items[0].bounds.center()));
    let knob = rotation_handle(&app.items[0], view).1;
    let button = |pos, pressed, modifiers| egui::Event::PointerButton {
      pos,
      pressed,
      button: PointerButton::Primary,
      modifiers,
    };
    frame(
      &mut app,
      vec![
        egui::Event::PointerMoved(knob),
        button(knob, true, Modifiers::NONE),
        egui::Event::PointerMoved(knob + Vec2::new(-24.0, -8.0)),
      ],
      Modifiers::NONE,
    );
    assert!(matches!(
      app.interaction,
      Some(CanvasInteraction::RotateItem { index: 0, .. })
    ));
    let a = 37.0_f32.to_radians();
    let target = center + Vec2::new(a.cos(), -a.sin()) * center.distance(knob);
    frame(
      &mut app,
      vec![egui::Event::PointerMoved(target)],
      Modifiers::NONE,
    );
    assert!((app.items[0].rotation.radians().to_degrees() - 37.0).abs() < 0.001);
    frame(&mut app, vec![], Modifiers::SHIFT);
    assert!((app.items[0].rotation.radians().to_degrees() - 45.0).abs() < 0.001);
    frame(&mut app, vec![], Modifiers::NONE);
    assert!((app.items[0].rotation.radians().to_degrees() - 37.0).abs() < 0.001);
    frame(
      &mut app,
      vec![button(target, false, Modifiers::NONE)],
      Modifiers::NONE,
    );
    assert!(app.interaction.is_none());
    assert_eq!(app.items[1].placed_bounds(), second);
    assert_eq!(format!("{:?}", app.items[0].primitives), source);
    assert_eq!(app.measurements.completed.len(), 1);
  }

  #[test]
  fn rotation_crossing_minus_pi_and_snapping_from_nonzero_angle_is_continuous() {
    let r = f64::to_radians;
    assert!(
      (rotation_angle(r(20.0), r(179.0), r(-179.0), false).to_degrees() - 22.0).abs() < 1.0e-9
    );
    assert!((rotation_angle(r(20.0), r(10.0), r(27.0), true).to_degrees() - 45.0).abs() < 1.0e-9);
  }

  #[test]
  fn background_import_continues_after_error_and_keeps_existing_measurements() {
    let context = egui::Context::default();
    let mut app = focus_test_app();
    app.items[0].rotation = crate::geometry::Rotation::new(0.7);
    let existing = format!("{:?}", app.items);
    let view_before = (app.view_center, app.zoom);
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
    assert!(!app.needs_layout && !app.needs_fit);
    assert_eq!(format!("{:?}", &app.items[..2]), existing);
    assert_eq!((app.view_center, app.zoom), view_before);
    assert!(
      app.items[2].placed_bounds().min.x
        > app.items[..2]
          .iter()
          .map(|item| item.placed_bounds().max.x)
          .fold(f64::NEG_INFINITY, f64::max)
    );
    assert!(app.world_bounds.unwrap().max.x >= app.items[2].placed_bounds().max.x);
    assert!(app.imports.progress().is_none());
  }

  #[test]
  fn removing_a_file_keeps_remaining_placement_rotation_dimensions_and_view() {
    let mut app = focus_test_app();
    app.items[1].rotation = crate::geometry::Rotation::new(0.8);
    app.selected_item = Some(1);
    let remaining = format!("{:?}", app.items[1]);
    let label = app.measurements.completed[0].label;
    let value = app.measurements.completed[0].value();
    let view = (app.view_center, app.zoom);
    app.remove_item(0);
    assert_eq!(format!("{:?}", app.items[0]), remaining);
    assert_eq!((app.view_center, app.zoom), view);
    assert_eq!(app.selected_item, Some(0));
    assert_eq!(app.measurements.completed[0].item, 0);
    assert_eq!(app.measurements.completed[0].label, label);
    assert_eq!(app.measurements.completed[0].value(), value);
    assert_eq!(app.world_bounds, Some(app.items[0].placed_bounds()));
    assert!(!app.needs_layout && !app.needs_fit);
    app.remove_item(0);
    assert!(app.items.is_empty() && app.world_bounds.is_none());
    assert!(app.measurements.completed.is_empty());
  }

  #[test]
  fn first_batch_is_fitted_once_without_rearranging_already_loaded_files() {
    let mut app = focus_test_app();
    app.items.clear();
    app.measurements.clear();
    app.diagnostics.clear();
    app.selected_item = None;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    app.add_paths(vec![
      root.join("examples/measurement_demo.dxf"),
      root.join("examples/advanced_demo.dxf"),
    ]);
    let context = egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut first = None;
    while app.imports.is_busy() {
      app.poll_imports(&context);
      assert!(!app.needs_layout);
      if app.imports.is_busy() {
        assert!(!app.needs_fit);
      }
      if let Some(item) = app.items.first() {
        if let Some(offset) = first {
          assert_eq!(item.offset, offset);
        }
        first = Some(item.offset);
      }
      assert!(std::time::Instant::now() < deadline);
      std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(app.items.len(), 2);
    assert!(app.needs_fit && !app.fit_initial_import);
    app.fit_all(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0)));
    app.poll_imports(&context);
    assert!(!app.needs_fit);
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
