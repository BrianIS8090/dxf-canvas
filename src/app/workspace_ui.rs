use super::*;

const ACCENT: Color32 = Color32::from_rgb(30, 94, 168);
const MUTED: Color32 = Color32::from_rgb(92, 105, 121);
const BORDER: Color32 = Color32::from_rgb(218, 224, 231);

#[derive(Clone, Copy, Default, PartialEq)]
enum InspectorTab {
  #[default]
  Files,
  Layers,
  Check,
}

#[cfg(test)]
mod tests {
  use super::*;

  fn frame(
    app: &mut DxfCanvasApp,
    context: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
  ) -> egui::FullOutput {
    let mut output = context.run_ui(
      egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 820.0))),
        events,
        ..Default::default()
      },
      |ui| {
        app.show_toolbar(ui);
        app.show_inspector(ui);
        app.show_status(ui);
        app.show_workspace_dialogs(context);
      },
    );
    output.textures_delta.clear();
    output
  }

  fn click_label(app: &mut DxfCanvasApp, context: &egui::Context, width: f32, label: &str) {
    let _ = frame(app, context, width, vec![]);
    let output = frame(app, context, width, vec![]);
    let point = output
      .shapes
      .iter()
      .find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
          Some(text.pos + text.galley.size() * 0.5)
        }
        _ => None,
      })
      .unwrap_or_else(|| panic!("Кнопка не найдена: {label}"));
    for pressed in [true, false] {
      let _ = frame(
        app,
        context,
        width,
        vec![
          egui::Event::PointerMoved(point),
          egui::Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
          },
        ],
      );
    }
  }

  #[test]
  fn inspector_tabs_preserve_active_measurement_and_completed_dimensions() {
    for width in [760.0, 1280.0] {
      let mut app = super::super::tests::focus_test_app();
      app.measurements.set_tool(Tool::Region);
      let context = egui::Context::default();
      configure_fonts_and_style(&context);
      for label in ["Слои", "Проверка", "Файлы"] {
        click_label(&mut app, &context, width, label);
        assert_eq!(app.measurements.tool, Tool::Region);
        assert_eq!(app.measurements.completed.len(), 1);
      }
    }
  }

  #[test]
  fn search_and_focus_change_only_the_view_not_source_or_measurements() {
    let mut app = super::super::tests::focus_test_app();
    app.workspace.file_filter = "нет такого файла".into();
    let before = (app.items[1].offset, app.items[1].scale);
    let context = egui::Context::default();
    let _ = frame(&mut app, &context, 1280.0, vec![]);
    assert_eq!(app.items.len(), 2);
    app.focus_item(1);
    app.focus_workspace_selection(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 500.0)));
    assert!(app.zoom.is_finite() && app.zoom > 0.0);
    assert_eq!(app.view_center, app.items[1].placed_bounds().center());
    assert_eq!((app.items[1].offset, app.items[1].scale), before);
    assert_eq!(app.measurements.completed.len(), 1);
  }

  #[test]
  fn cancelled_clear_keeps_all_files_and_measurements() {
    let mut app = super::super::tests::focus_test_app();
    app.workspace.clear_target = Some(ClearTarget::Canvas);
    let context = egui::Context::default();
    click_label(&mut app, &context, 1280.0, "Оставить");
    assert!(app.workspace.clear_target.is_none());
    assert_eq!(app.items.len(), 2);
    assert_eq!(app.measurements.completed.len(), 1);
  }

  #[test]
  fn file_names_wrap_left_aligned_with_the_metadata() {
    let mut app = super::super::tests::focus_test_app();
    app.items[0].name = "2 мм (ст3)_13 шт_26-122_Уголок крепежный с длинным названием".into();
    let context = egui::Context::default();
    configure_fonts_and_style(&context);
    let _ = frame(&mut app, &context, 760.0, vec![]);
    let output = frame(&mut app, &context, 760.0, vec![]);
    let title = output
      .shapes
      .iter()
      .find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == app.items[0].name => Some(text),
        _ => None,
      })
      .expect("Название файла должно отображаться");
    assert!(title.galley.rows.len() > 1);
    assert!(
      !title.galley.job.justify,
      "Название нельзя растягивать по ширине строки"
    );
    for row in &title.galley.rows {
      assert!(
        row.pos.x.abs() < 0.1,
        "Строка названия смещена от левого края"
      );
    }
    let metadata = output
      .shapes
      .iter()
      .find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text().starts_with("100.0 × 50.0") => Some(text),
        _ => None,
      })
      .unwrap();
    assert!((title.pos.x - metadata.pos.x).abs() < 0.1);
  }

  #[test]
  fn arrange_button_is_available_without_a_menu_and_preserves_scales_and_rotations() {
    for width in [760.0, 1280.0] {
      let mut app = super::super::tests::focus_test_app();
      app.items[1].rotation = crate::geometry::Rotation::new(0.6);
      let rotation = app.items[1].rotation.radians();
      let scale = app.items[1].scale;
      let before = app.items[1].offset;
      let context = egui::Context::default();
      configure_fonts_and_style(&context);
      click_label(&mut app, &context, width, "Разложить");
      assert!(app.needs_layout && app.needs_fit);
      app.perform_layout(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 500.0)));
      assert_ne!(app.items[1].offset, before);
      assert_eq!(app.items[1].rotation.radians(), rotation);
      assert_eq!(app.items[1].scale, scale);
      assert_eq!(app.measurements.completed.len(), 1);
    }
  }

  #[test]
  fn long_file_names_do_not_expand_the_layers_panel() {
    let mut app = super::super::tests::focus_test_app();
    app.items[0].name = "3 мм (амг2м)_13 шт_26-122_Основание под установку источника света_ОченьДлинноеНазваниеБезПробелов".repeat(3);
    let context = egui::Context::default();
    configure_fonts_and_style(&context);
    for width in [240.0, 280.0, 360.0, 440.0] {
      let mut used_width = 0.0;
      let mut output = context.run_ui(
        egui::RawInput {
          screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 800.0))),
          ..Default::default()
        },
        |ui| {
          ui.set_width(width);
          app.layers_panel(ui);
          used_width = ui.min_rect().width();
        },
      );
      output.textures_delta.clear();
      assert!(
        used_width <= width + 1.0,
        "Панель шириной {width} раздвинулась до {used_width}"
      );
    }
  }

  #[test]
  fn file_cards_with_long_names_keep_panel_width_and_natural_letter_spacing() {
    let context = egui::Context::default();
    configure_fonts_and_style(&context);
    for width in [240.0, 280.0, 360.0, 440.0] {
      for name in [
        "2 мм (ст3)_7 шт_26-122_Усиливающая шайба.",
        "3 мм (амг2м)_13 шт_26-122_Основание под установку источника света.",
        "ОченьДлинноеНазваниеДеталиБезПробелов_ОченьДлинноеНазваниеДеталиБезПробелов",
      ] {
        let mut app = super::super::tests::focus_test_app();
        app.items[0].name = name.into();
        let mut used_width = 0.0;
        let mut output = context.run_ui(
          egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 1000.0))),
            ..Default::default()
          },
          |ui| {
            ui.set_width(width);
            app.files_panel(ui);
            used_width = ui.min_rect().width();
          },
        );
        output.textures_delta.clear();
        assert!(
          used_width <= width + 1.0,
          "Карточка раздвинула панель: {used_width} > {width}"
        );
        let text = output
          .shapes
          .iter()
          .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == name => Some(text),
            _ => None,
          })
          .unwrap();
        assert!(!text.galley.job.justify);
        assert!(
          !text.galley.elided,
          "В карточке должно оставаться полное имя"
        );
        for row in &text.galley.rows {
          assert!(row.pos.x.abs() < 0.1);
        }
      }
    }
  }

  #[test]
  fn long_file_picker_menu_is_bounded_and_can_select_another_file() {
    let context = egui::Context::default();
    configure_fonts_and_style(&context);
    let mut app = super::super::tests::focus_test_app();
    app.items[0].name = "Имя первого файла с длинным названием_".repeat(5);
    app.items[1].name = "Имя второго файла с длинным названием_".repeat(5);
    let mut render = |events| {
      let mut rect = Rect::NOTHING;
      let mut output = context.run_ui(
        egui::RawInput {
          screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 800.0))),
          events,
          ..Default::default()
        },
        |ui| {
          ui.set_width(260.0);
          app.selected_file_picker(ui);
          rect = ui.min_rect();
        },
      );
      output.textures_delta.clear();
      (rect, output)
    };
    let (rect, _) = render(vec![]);
    let point = rect.center_top() + Vec2::new(0.0, 10.0);
    let pointer_event = |pos, pressed| egui::Event::PointerButton {
      pos,
      button: PointerButton::Primary,
      pressed,
      modifiers: Modifiers::NONE,
    };
    render(vec![
      egui::Event::PointerMoved(point),
      pointer_event(point, true),
    ]);
    render(vec![pointer_event(point, false)]);
    let (_, output) = render(vec![]);
    let text = output
      .shapes
      .iter()
      .find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text().starts_with("Имя второго файла") => {
          Some(text)
        }
        _ => None,
      })
      .expect("Список выбора должен открываться");
    assert!(text.galley.elided);
    assert!(text.galley.size().x < 260.0);
    let point = text.pos + text.galley.size() * 0.5;
    render(vec![
      egui::Event::PointerMoved(point),
      pointer_event(point, true),
    ]);
    render(vec![pointer_event(point, false)]);
    assert_eq!(app.selected_item, Some(1));
  }
}

#[derive(Clone, Copy)]
enum ClearTarget {
  Canvas,
  Dimensions,
}

pub(super) struct WorkspaceUi {
  tab: InspectorTab,
  file_filter: String,
  pub(super) help_open: bool,
  clear_target: Option<ClearTarget>,
  inspector_visible: bool,
  focus_bounds: Option<Bounds>,
}

impl Default for WorkspaceUi {
  fn default() -> Self {
    Self {
      tab: InspectorTab::Files,
      file_filter: String::new(),
      help_open: false,
      clear_target: None,
      inspector_visible: true,
      focus_bounds: None,
    }
  }
}

impl DxfCanvasApp {
  fn toggle_check(&mut self) {
    if self.diagnostics.enabled {
      self.diagnostics.clear();
    } else if !self.items.is_empty() {
      self.diagnostics.enabled = true;
      self.checking.request();
      self.interaction = None;
      self.workspace.tab = InspectorTab::Check;
      self.workspace.inspector_visible = true;
    }
  }

  pub(super) fn show_toolbar(&mut self, root: &mut egui::Ui) {
    egui::Panel::top("toolbar")
      .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(10.0))
      .show(root, |ui| {
        ui.horizontal_wrapped(|ui| {
          ui.heading(RichText::new("DXF Холст").size(20.0).color(ACCENT));
          ui.menu_button(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).small(), |ui| {
            ui.set_max_width(450.0);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
              ui.label(include_str!("../../THIRD_PARTY_NOTICES.md"));
              ui.collapsing("Лицензия приложения", |ui| { ui.label(include_str!("../../LICENSE")); });
              ui.collapsing("ACadSharp / CSUtilities", |ui| { ui.label(include_str!("../../docs/licenses/ACADSHARP-LICENSE.txt")); });
              ui.collapsing(".NET NativeAOT", |ui| {
                ui.label(include_str!("../../docs/licenses/DOTNET-LICENSE.txt"));
                ui.label(include_str!("../../docs/licenses/DOTNET-NATIVE-NOTICES.txt"));
              });
            });
          });
          ui.separator();
          if ui.add(egui::Button::new(RichText::new("Добавить DWG / DXF").color(Color32::WHITE)).fill(ACCENT))
            .on_hover_text("Выбрать один или несколько файлов · Ctrl+O\nТакже можно перетащить файлы в окно. Новые файлы появятся справа от существующих; «Вписать всё» покажет их без перестановки.").clicked() {
            self.choose_files();
          }
          if ui.add_enabled(!self.items.is_empty(), egui::Button::new("Вписать всё"))
            .on_hover_text("Показать все чертежи · Ctrl+0").clicked() {
            self.needs_fit = true;
          }
          if ui.add_enabled(!self.items.is_empty(), egui::Button::new("Разложить"))
            .on_hover_text("Автоматически расположить все файлы и вписать их в окно. Текущая ручная раскладка будет заменена; углы поворота и масштабы сохранятся.").clicked() {
            self.needs_layout = true;
            self.needs_fit = true;
          }
          ui.menu_button("Холст", |ui| {
            ui.checkbox(&mut self.workspace.inspector_visible, "Боковая панель");
            ui.separator();
            ui.label(RichText::new("Подписи на холсте").strong());
            ui.add(egui::Slider::new(&mut self.label_font_size, 8.0..=36.0).step_by(1.0).suffix(" px"));
            ui.label(RichText::new("Один размер для всех файлов и измерений").small());
            ui.separator();
            if ui.add_enabled(!self.items.is_empty(), egui::Button::new("Очистить холст…")).clicked() {
              self.workspace.clear_target = Some(ClearTarget::Canvas);
              ui.close();
            }
          });
          if ui.add_enabled(!self.items.is_empty(), egui::Button::new(if self.diagnostics.enabled { "Проверка включена" } else { "Проверить" }).selected(self.diagnostics.enabled))
            .on_hover_text("Подсветить проблемные места и открыть сводку. Повторное нажатие выключает подсветку. Исходный файл не изменяется.").clicked() {
            self.toggle_check();
          }
          if ui.button("Справка · F1").clicked() {
            self.workspace.help_open = true;
          }
        });
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal_wrapped(|ui| {
          for (tool, label, description) in [
            (Tool::Select, "Выбор · V", "Перемещать и масштабировать файлы на холсте"),
            (Tool::Linear, "Линейный · L", "Две точки с привязками, затем положение размера"),
            (Tool::Diameter, "Диаметр · D", "Диаметр круглого отверстия"),
            (Tool::Radius, "Радиус · R", "Радиус круговой дуги скругления"),
            (Tool::Angle, "Угол · G", "Первая точка, вершина, третья точка"),
            (Tool::Region, "Площадь · A", "Площадь и периметр детали или отдельного контура"),
          ] {
            if ui.add_enabled(!self.items.is_empty() || tool == Tool::Select,
              egui::Button::new(label).selected(self.measurements.tool == tool))
              .on_hover_text(description).clicked() {
              self.measurements.set_tool(tool);
              self.interaction = None;
            }
          }
          ui.separator();
          if ui.add_enabled(self.measurements.can_undo(), egui::Button::new("Отменить"))
            .on_hover_text("Ctrl+Z — отменить текущий или последний размер").clicked() {
            self.measurements.undo();
          }
          ui.menu_button(format!("Размеры: {}", self.measurements.completed.len()), |ui| {
            ui.label("Размеры рассчитаны по исходной геометрии.");
            ui.label("Масштаб детали на холсте на результат не влияет.");
            if ui.add_enabled(!self.measurements.completed.is_empty(), egui::Button::new("Убрать все размеры…")).clicked() {
              self.workspace.clear_target = Some(ClearTarget::Dimensions);
              ui.close();
            }
          });
        });
        if self.measurements.tool == Tool::Region {
          ui.add_space(5.0);
          egui::Frame::new().fill(Color32::from_rgb(240, 245, 251)).corner_radius(5.0).inner_margin(8.0).show(ui, |ui| {
            region_mode_controls(ui, &mut self.measurements);
            ui.label(RichText::new(if self.measurements.contour_only {
              "Для планов: один замкнутый контур под курсором. Отверстия внутри не вычитаются."
            } else {
              "Для деталей: площадь за вычетом отверстий. Периметр включает отверстия и прорези."
            }).small().color(MUTED));
          });
        }
      });
  }

  pub(super) fn show_inspector(&mut self, root: &mut egui::Ui) {
    if self.items.is_empty() || !self.workspace.inspector_visible {
      return;
    }
    egui::Panel::right("files")
      .default_size(320.0)
      .min_size(280.0)
      .max_size(460.0)
      .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(12.0))
      .show(root, |ui| {
        ui.horizontal_wrapped(|ui| {
          ui.heading("Рабочая область");
          ui.label(
            RichText::new(format!("{} файлов", self.items.len()))
              .small()
              .color(MUTED),
          );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
          ui.selectable_value(&mut self.workspace.tab, InspectorTab::Files, "Файлы");
          ui.selectable_value(&mut self.workspace.tab, InspectorTab::Layers, "Слои");
          ui.selectable_value(&mut self.workspace.tab, InspectorTab::Check, "Проверка");
        });
        ui.separator();
        match self.workspace.tab {
          InspectorTab::Files => self.files_panel(ui),
          InspectorTab::Layers => self.layers_panel(ui),
          InspectorTab::Check => self.check_panel(ui),
        }
      });
  }

  fn files_panel(&mut self, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
      let width = (ui.available_width() - 38.0).max(80.0);
      ui.add(
        egui::TextEdit::singleline(&mut self.workspace.file_filter)
          .hint_text("Найти файл по названию")
          .desired_width(width),
      );
      if ui.button("×").on_hover_text("Сбросить поиск").clicked() {
        self.workspace.file_filter.clear();
      }
    });
    ui.label(
      RichText::new("Выберите файл; двойной щелчок — приблизить")
        .small()
        .color(MUTED),
    );
    ui.add_space(6.0);
    let query = self.workspace.file_filter.trim().to_lowercase();
    let mut remove = None;
    let mut focus = None;
    let mut changed = false;
    let mut matches = 0;
    egui::ScrollArea::vertical()
      .id_salt("file_list")
      .show(ui, |ui| {
        for (index, item) in self.items.iter_mut().enumerate() {
          if !query.is_empty() && !item.name.to_lowercase().contains(&query) {
            continue;
          }
          matches += 1;
          let selected = self.selected_item == Some(index);
          egui::Frame::new()
            .fill(if selected {
              Color32::from_rgb(238, 245, 253)
            } else {
              Color32::from_rgb(248, 250, 252)
            })
            .stroke(Stroke::new(1.0, if selected { ACCENT } else { BORDER }))
            .corner_radius(6.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
              ui.horizontal_top(|ui| {
                let width = (ui.available_width() - 38.0).max(60.0);
                let response = ui
                  .allocate_ui_with_layout(
                    Vec2::new(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                      // Резервируем место перед крестиком, не растягивая текст по ширине.
                      ui.set_min_width(width);
                      ui.add(
                        egui::Label::new(RichText::new(&item.name).strong())
                          .wrap()
                          .halign(egui::Align::Min)
                          .selectable(false)
                          .sense(Sense::click()),
                      )
                    },
                  )
                  .inner
                  .on_hover_cursor(egui::CursorIcon::PointingHand)
                  .on_hover_text(item.path.display().to_string());
                if response.clicked() {
                  self.selected_item = Some(index);
                }
                if response.double_clicked() {
                  self.selected_item = Some(index);
                  focus = Some(index);
                }
                if ui
                  .button("×")
                  .on_hover_text(
                    "Убрать с холста вместе с размерами этого файла. Файл на диске останется.",
                  )
                  .clicked()
                {
                  remove = Some(index);
                }
              });
              ui.label(
                RichText::new(format!(
                  "{:.1} × {:.1} {}",
                  item.bounds.width() * item.units.factor(),
                  item.bounds.height() * item.units.factor(),
                  item.units.label()
                ))
                .small()
                .color(MUTED),
              );
              ui.label(
                RichText::new(format!(
                  "{} элементов · {} слоёв",
                  item.primitives.len() + item.appearance.texts.len() + item.appearance.fills.len(),
                  item.appearance.layers.len()
                ))
                .small()
                .color(MUTED),
              );
              if item.unsupported_entities > 0 {
                ui.colored_label(
                  Color32::from_rgb(155, 88, 13),
                  format!("Не показано сущностей: {}", item.unsupported_entities),
                );
              }
              if selected {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                  if ui.button("Приблизить").clicked() {
                    focus = Some(index);
                  }
                  if ui.button("Слои файла").clicked() {
                    self.workspace.tab = InspectorTab::Layers;
                  }
                });
                ui.separator();
                ui.label(RichText::new("Масштаб на холсте").small().strong());
                ui.horizontal(|ui| {
                  if ui
                    .add(
                      egui::Slider::new(&mut item.scale, MIN_ITEM_SCALE..=MAX_ITEM_SCALE)
                        .logarithmic(true)
                        .show_value(false),
                    )
                    .changed()
                  {
                    changed = true;
                  }
                  ui.label(format!("{:.0}%", item.scale * 100.0));
                });
                ui.horizontal(|ui| {
                  if ui.button("−").clicked() {
                    item.scale = (item.scale / 1.1).max(MIN_ITEM_SCALE);
                    changed = true;
                  }
                  if ui.button("100%").clicked() {
                    item.scale = 1.0;
                    changed = true;
                  }
                  if ui.button("+").clicked() {
                    item.scale = (item.scale * 1.1).min(MAX_ITEM_SCALE);
                    changed = true;
                  }
                });
                ui.label(
                  RichText::new("Исходные размеры не меняются")
                    .small()
                    .color(MUTED),
                );
                if !item.appearance.warnings.is_empty() {
                  ui.collapsing(
                    format!("Импорт: {} предупреждений", item.appearance.warnings.len()),
                    |ui| {
                      for warning in &item.appearance.warnings {
                        ui.colored_label(Color32::from_rgb(155, 88, 13), warning);
                      }
                    },
                  );
                }
              }
            });
          ui.add_space(8.0);
        }
        if matches == 0 {
          ui.label("Файлы не найдены. Измените или сбросьте поиск.");
        }
      });
    if changed {
      self.recalculate_world_bounds();
    }
    if let Some(index) = focus {
      self.focus_item(index);
    }
    if let Some(index) = remove {
      self.remove_item(index);
    }
  }

  fn selected_file_picker(&mut self, ui: &mut egui::Ui) -> Option<usize> {
    let mut index = self
      .selected_item
      .filter(|&i| i < self.items.len())
      .unwrap_or(0);
    let width = ui.available_width();
    egui::ComboBox::from_id_salt("inspector_file")
      .width(width)
      .truncate()
      .selected_text(&self.items[index].name)
      .show_ui(ui, |ui| {
        ui.set_max_width(width);
        for (i, item) in self.items.iter().enumerate() {
          if ui
            .add(egui::Button::selectable(index == i, &item.name).truncate())
            .on_hover_text(&item.name)
            .clicked()
          {
            index = i;
          }
        }
      })
      .response
      .on_hover_text(&self.items[index].name);
    if self.selected_item != Some(index) {
      self.selected_item = Some(index);
      self.layer_filter.clear();
    }
    ui.add_space(6.0);
    Some(index)
  }

  fn layers_panel(&mut self, ui: &mut egui::Ui) {
    let Some(index) = self.selected_file_picker(ui) else {
      return;
    };
    ui.label(
      RichText::new("Видимость слоёв выбранного файла")
        .small()
        .color(MUTED),
    );
    ui.label(
      RichText::new("Скрытые слои не участвуют в новых измерениях и проверке.")
        .small()
        .color(MUTED),
    );
    ui.add_space(8.0);
    if crate::cad_render::layers_ui(ui, &mut self.items[index], &mut self.layer_filter) {
      self.diagnostics.clear_selection();
      if self.diagnostics.enabled {
        self.diagnostics.reports.clear();
        self.checking.request();
      }
      // При смене видимости отменяем только построение, сохраняя выбранный инструмент.
      let tool = self.measurements.tool;
      self.measurements.set_tool(tool);
    }
  }

  fn check_panel(&mut self, ui: &mut egui::Ui) {
    if !self.diagnostics.enabled {
      ui.add_space(12.0);
      ui.heading("Проверка геометрии");
      ui.label("Разрывы, совпадения, пересечения и другие места, которые стоит проверить перед производством.");
      ui.add_space(8.0);
      if ui.button("Запустить проверку").clicked() {
        self.toggle_check();
      }
      ui.label(
        RichText::new("Исходные DWG и DXF не изменяются.")
          .small()
          .color(MUTED),
      );
      return;
    }
    egui::ScrollArea::vertical()
      .id_salt("check_panel")
      .show(ui, |ui| {
        show_legend(ui, &mut self.diagnostics);
        for (index, item) in self.items.iter().enumerate() {
          ui.label(RichText::new(&item.name).strong());
          if let Some(report) = self.diagnostics.reports.get(index) {
            let selected = self
              .diagnostics
              .selected
              .filter(|s| s.item == index)
              .map(|s| s.finding);
            if let Some(finding) =
              show_file_report(ui, report, index, selected, self.diagnostics.filter)
            {
              self.selected_item = Some(index);
              self.diagnostics.select(index, finding);
            }
          }
          ui.separator();
        }
      });
  }

  fn focus_item(&mut self, index: usize) {
    if let Some(item) = self.items.get(index) {
      let bounds = item.placed_bounds();
      self.view_center = bounds.center();
      // Точный размер доступного холста учитывается при следующей отрисовке.
      self.workspace.focus_bounds = Some(bounds);
      self.diagnostics.clear_selection();
      self.interaction = None;
    }
  }

  pub(super) fn focus_workspace_selection(&mut self, canvas: Rect) {
    if let Some(bounds) = self.workspace.focus_bounds.take() {
      self.zoom = ((canvas.width() - 2.0 * CANVAS_PADDING).max(1.0)
        / bounds.width().max(1.0e-9) as f32)
        .min((canvas.height() - 2.0 * CANVAS_PADDING).max(1.0) / bounds.height().max(1.0e-9) as f32)
        .clamp(MIN_ZOOM, MAX_ZOOM);
      self.view_center = bounds.center();
      self.needs_fit = false;
    }
  }

  pub(super) fn workspace_dialog_open(&self) -> bool {
    self.workspace.help_open || self.workspace.clear_target.is_some()
  }

  pub(super) fn show_status(&mut self, root: &mut egui::Ui) {
    egui::Panel::bottom("workspace_status")
      .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(8.0))
      .show(root, |ui| {
        ui.horizontal_wrapped(|ui| {
          if !self.items.is_empty()
            && !self.workspace.inspector_visible
            && ui.button("Показать панель").clicked()
          {
            self.workspace.inspector_visible = true;
          }
          ui.label(
            RichText::new(
              self
                .measurements
                .notice
                .as_deref()
                .unwrap_or(self.measurements.hint()),
            )
            .color(if self.measurements.notice.is_some() {
              Color32::from_rgb(156, 80, 13)
            } else {
              MUTED
            }),
          );
        });
      });
  }

  pub(super) fn show_workspace_dialogs(&mut self, context: &egui::Context) {
    egui::Window::new("Управление и измерения").open(&mut self.workspace.help_open).collapsible(false).resizable(true).default_width(550.0).show(context, |ui| {
      egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
        ui.heading("Холст");
        for (key, action) in [
          ("Ctrl+O", "Добавить DWG / DXF"), ("Ctrl+0", "Вписать все файлы"),
          ("Колесо", "Приблизить участок под курсором"),
            ("Средняя кнопка мыши", "Перемещать холст"),
          ("V · левая кнопка мыши", "Выбрать и двигать файл"),
          ("Ctrl+колесо / угловые маркеры", "Масштабировать файл на холсте"),
          ("Круглый маркер справа", "Повернуть файл вокруг центра; Shift — шаг 45°"),
          ("L / D / R / G / A", "Линейный / диаметр / радиус / угол / площадь"),
          ("Ctrl+Z", "Отменить текущий или последний размер"), ("Esc", "Отменить построение; повторно — вернуться к выбору"),
        ] { ui.horizontal_wrapped(|ui| { ui.label(RichText::new(key).strong()); ui.label(action); }); }
        ui.separator();
        ui.heading("Привязки");
        ui.label("Линейный размер: выберите первую точку, затем вторую и место подписи. Концы, середины, центры и квадранты определяются автоматически. После первой точки доступна привязка «Перпендикуляр» к линии.");
        ui.separator();
        ui.heading("Два режима площади");
        ui.label("Деталь с отверстиями: вычитает отверстия из площади, учитывает внутренние границы и прорези в периметре.");
        ui.label("Отдельный контур: выбирает один замкнутый объект под курсором без вычитания вложенных отверстий. Подходит для элементов на архитектурном плане.");
        ui.label("Щёлкните внутри области, затем разместите подпись. Площадь — в м², периметр — в м, если единицы файла известны.");
        ui.separator();
        ui.label(RichText::new("Масштаб на холсте не изменяет исходную геометрию и результаты измерений. Скрытые слои исключены из новых измерений и проверки.").color(ACCENT));
      });
    });
    if let Some(target) = self.workspace.clear_target {
      egui::Modal::new(egui::Id::new("confirm_clear")).show(context, |ui| {
        ui.set_max_width(420.0);
        ui.heading(match target { ClearTarget::Canvas => "Очистить холст?", ClearTarget::Dimensions => "Убрать все размеры?" });
        ui.label(match target {
          ClearTarget::Canvas => "Файлы будут убраны с холста вместе с расстановкой и измерениями. Исходные файлы на диске останутся.",
          ClearTarget::Dimensions => "Все размещённые размеры будут убраны. Чертежи и их расположение останутся.",
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
          if ui.button("Оставить").clicked() { self.workspace.clear_target = None; }
          if ui.button("Очистить").clicked() {
            if matches!(target, ClearTarget::Canvas) {
              self.items.clear(); self.world_bounds = None; self.selected_item = None;
              self.interaction = None; self.diagnostics.clear(); self.workspace.file_filter.clear();
            }
            self.measurements.clear();
            self.workspace.clear_target = None;
          }
        });
      });
    }
  }
}
