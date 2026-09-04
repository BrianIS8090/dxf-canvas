use eframe::egui::{self, Color32, Painter, Rect, RichText, Stroke, StrokeKind, Vec2};

use crate::{
  diagnostics::{DiagnosticReport, DiagnosticsState, Finding, IssueKind, Marker},
  geometry::{DrawingItem, MeasureCurve, Point, Primitive, ViewTransform},
};

fn color(kind: IssueKind) -> Color32 {
  match kind {
    IssueKind::OpenContour => Color32::from_rgb(207, 43, 52),
    IssueKind::UnjoinedContour => Color32::from_rgb(0, 132, 119),
    IssueKind::Duplicate => Color32::from_rgb(189, 35, 137),
    IssueKind::ShortSegment => Color32::from_rgb(203, 111, 12),
    IssueKind::Oval => Color32::from_rgb(112, 66, 188),
    IssueKind::UnknownUnits => Color32::from_rgb(159, 115, 6),
    IssueKind::IncompleteGeometry => Color32::from_rgb(55, 109, 145),
    IssueKind::Intersection => Color32::from_rgb(215, 48, 20),
    IssueKind::PartialOverlap => Color32::from_rgb(174, 42, 170),
  }
}

pub fn show_legend(ui: &mut egui::Ui, state: &mut DiagnosticsState) {
  ui.heading("Проверка DXF");
  let mut filter = state.filter;
  egui::ComboBox::from_id_salt("diagnostics_category")
    .selected_text(filter.map_or("Все категории", IssueKind::label))
    .width(ui.available_width().min(260.0))
    .show_ui(ui, |ui| {
      ui.selectable_value(&mut filter, None, "Все категории");
      for kind in IssueKind::ALL {
        ui.selectable_value(&mut filter, Some(kind), kind.label());
      }
    });
  if filter != state.filter {
    state.set_filter(filter);
  }
  let (position, count) = state.navigation_position();
  ui.horizontal(|ui| {
    if ui
      .add_enabled(count > 0, egui::Button::new("← Назад"))
      .clicked()
    {
      state.navigate(false);
    }
    if ui
      .add_enabled(count > 0, egui::Button::new("Далее →"))
      .clicked()
    {
      state.navigate(true);
    }
    ui.label(format!("{position} / {count}"));
  });
  let reports = &state.reports;
  let total: usize = reports.iter().map(|report| report.findings.len()).sum();
  if total == 0 {
    ui.colored_label(
      Color32::from_rgb(27, 121, 83),
      "По этим проверкам замечаний нет",
    );
  } else {
    ui.label(format!("Мест для проверки: {total}"));
  }
  ui.label(RichText::new("Легенда подсветки").strong());
  for kind in IssueKind::ALL {
    let count: usize = reports.iter().map(|report| report.count(kind)).sum();
    ui.horizontal(|ui| {
      ui.colored_label(color(kind), "●");
      ui.label(format!("{}: {count}", kind.label()))
        .on_hover_text(kind.explanation());
    });
  }
  ui.label(RichText::new("Стыки: ≤ 0,01 мм · короткие: < 0,1 мм").small())
    .on_hover_text("Для файлов без единиц используются те же числовые пороги в ед. DXF. Дубли: полное совпадение с допуском 0,001 мм.");
  ui.label(RichText::new("Это предупреждения. DXF не изменяется.").small());
  ui.label(RichText::new("Повторное нажатие кнопки выключит подсветку.").small());
  ui.separator();
}

pub fn show_file_report(
  ui: &mut egui::Ui,
  report: &DiagnosticReport,
  index: usize,
  selected: Option<usize>,
  filter: Option<IssueKind>,
) -> Option<usize> {
  if report.findings.is_empty() {
    ui.label(
      RichText::new("По проверке замечаний нет")
        .small()
        .color(Color32::from_rgb(27, 121, 83)),
    );
    return None;
  }
  let mut clicked = None;
  let filtered: Vec<_> = report
    .findings
    .iter()
    .enumerate()
    .filter(|(_, f)| filter.is_none_or(|k| k == f.kind))
    .collect();
  let selection_id = ui.make_persistent_id(("diagnostics_last_selection", index));
  let previous = ui
    .data(|d| d.get_temp::<Option<usize>>(selection_id))
    .flatten();
  ui.data_mut(|d| d.insert_temp(selection_id, selected));
  egui::CollapsingHeader::new(format!(
    "Замечания: {} / {}",
    filtered.len(),
    report.findings.len()
  ))
  .id_salt(("dxf_check", index))
  .open((selected.is_some() && previous != selected).then_some(true))
  .default_open(selected.is_some())
  .show(ui, |ui| {
    if filtered.is_empty() {
      ui.label("Нет замечаний выбранной категории");
      return;
    }
    ui.label(RichText::new("Нажмите на замечание — показать на холсте").small());
    let limit_id =
      ui.make_persistent_id(("diagnostics_visible_count", index, filter.map(|k| k as u8)));
    let shown = ui
      .data_mut(|data| *data.get_temp_mut_or(limit_id, 50_usize))
      .min(filtered.len());
    for &(finding_index, finding) in filtered.iter().take(shown) {
      if finding_button(ui, finding, finding_index, selected == Some(finding_index)).clicked() {
        clicked = Some(finding_index);
      }
    }
    if let Some(selected) = selected
      && let Some((_, finding)) = filtered.iter().skip(shown).find(|(i, _)| *i == selected)
    {
      ui.label("Текущее замечание:");
      if finding_button(ui, finding, selected, true).clicked() {
        clicked = Some(selected);
      }
    }
    if shown < filtered.len()
      && ui
        .button(format!(
          "Показать ещё {} замечаний",
          (filtered.len() - shown).min(50)
        ))
        .clicked()
    {
      ui.data_mut(|data| data.insert_temp(limit_id, shown.saturating_add(50)));
    }
  });
  clicked
}

fn finding_button(
  ui: &mut egui::Ui,
  finding: &Finding,
  index: usize,
  selected: bool,
) -> egui::Response {
  ui.add(
    egui::Button::new(
      RichText::new(format!(
        "{}. {} — {}",
        index + 1,
        finding.kind.label(),
        finding.detail
      ))
      .small()
      .color(color(finding.kind)),
    )
    .wrap()
    .selected(selected)
    .frame(selected),
  )
  .on_hover_cursor(egui::CursorIcon::PointingHand)
  .on_hover_text("Приблизить и выделить этот участок. Повторный щелчок снова центрирует вид.")
}

pub fn paint_report(
  painter: &Painter,
  item: &DrawingItem,
  report: &DiagnosticReport,
  transform: ViewTransform,
  filter: Option<IssueKind>,
) {
  let screen = |point| transform.world_to_screen(item.world_point(point));
  // Общие предупреждения рисуем за локальными маркерами, чтобы разрывы оставались видны.
  for (frame_index, kind) in IssueKind::ALL
    .into_iter()
    .filter(|kind| {
      filter.is_none_or(|k| k == *kind)
        && report
          .findings
          .iter()
          .any(|finding| finding.kind == *kind && matches!(finding.marker, Marker::File))
    })
    .enumerate()
  {
    let bounds = item.bounds;
    let rect = Rect::from_two_pos(screen(bounds.min), screen(bounds.max))
      .expand(4.0 + frame_index as f32 * 4.0);
    painter.rect_stroke(
      rect,
      4.0,
      Stroke::new(1.6, color(kind)),
      StrokeKind::Outside,
    );
  }
  // Цепочки рисуем первыми, чтобы совпадения и короткие участки оставались поверх них.
  for finding in report
    .findings
    .iter()
    .filter(|finding| matches!(finding.marker, Marker::Contour(_)))
    .chain(
      report
        .findings
        .iter()
        .filter(|finding| !matches!(finding.marker, Marker::Contour(_))),
    )
  {
    if filter.is_some_and(|k| k != finding.kind) {
      continue;
    }
    let stroke = Stroke::new(2.6, color(finding.kind));
    paint_marker(
      painter,
      item,
      &finding.marker,
      &screen,
      stroke,
      finding.kind == IssueKind::ShortSegment,
    );
  }
}

fn paint_marker(
  painter: &Painter,
  item: &DrawingItem,
  marker: &Marker,
  screen: &impl Fn(Point) -> egui::Pos2,
  stroke: Stroke,
  short: bool,
) {
  match *marker {
    Marker::Span(shape) => paint_path(painter, &shape.points(), false, screen, stroke),
    Marker::File => {}
    Marker::Point(point) => {
      let p = screen(point);
      painter.circle(p, 7.0, Color32::from_white_alpha(230), stroke);
      painter.line_segment([p - Vec2::new(3.0, 3.0), p + Vec2::new(3.0, 3.0)], stroke);
      painter.line_segment([p - Vec2::new(3.0, -3.0), p + Vec2::new(3.0, -3.0)], stroke);
    }
    Marker::Primitive(index) => {
      if let Some(Primitive::Path { points, closed, .. }) = item.primitives.get(index) {
        paint_path(painter, points, *closed, screen, stroke);
      }
    }
    Marker::Contour(ref indices) => {
      for index in indices {
        if let Some(Primitive::Path { points, closed, .. }) = item.primitives.get(*index) {
          paint_path(painter, points, *closed, screen, stroke);
        }
      }
    }
    Marker::Curve { primitive, curve } => {
      if let Some(Primitive::Path { curves, .. }) = item.primitives.get(primitive)
        && let Some(curve) = curves.get(curve)
      {
        match curve {
          MeasureCurve::Line { start, end } => {
            let (a, b) = (screen(*start), screen(*end));
            painter.line_segment([a, b], stroke);
            if a.distance(b) < 14.0 {
              painter.rect_stroke(
                Rect::from_center_size(a.lerp(b, 0.5), Vec2::splat(12.0)),
                1.0,
                stroke,
                StrokeKind::Middle,
              );
            }
          }
          MeasureCurve::Round(curve) => {
            let points: Vec<_> = (0..=96)
              .map(|i| curve.point_at(curve.start + curve.sweep * i as f64 / 96.0))
              .collect();
            paint_path(painter, &points, curve.is_full(), screen, stroke);
            if short {
              painter.circle_stroke(
                screen(curve.point_at(curve.start + curve.sweep * 0.5)),
                7.0,
                stroke,
              );
            }
          }
          MeasureCurve::Polyline { points, closed } => {
            paint_path(painter, points, *closed, screen, stroke);
            if short && let Some(point) = points.first() {
              painter.circle_stroke(screen(*point), 7.0, stroke);
            }
          }
        }
      }
    }
  }
}

pub fn paint_selected_finding(
  painter: &Painter,
  item: &DrawingItem,
  finding: &Finding,
  index: usize,
  transform: ViewTransform,
  canvas: Rect,
) {
  let Some(bounds) = finding.marker.bounds(item) else {
    return;
  };
  let screen = |point| transform.world_to_screen(item.world_point(point));
  let region = Rect::from_two_pos(screen(bounds.min), screen(bounds.max));
  let selected_color = color(finding.kind);
  // Белый ореол отделяет активное замечание от соседних цветных предупреждений.
  paint_marker(
    painter,
    item,
    &finding.marker,
    &screen,
    Stroke::new(8.0, Color32::WHITE),
    finding.kind == IssueKind::ShortSegment,
  );
  paint_marker(
    painter,
    item,
    &finding.marker,
    &screen,
    Stroke::new(4.0, selected_color),
    finding.kind == IssueKind::ShortSegment,
  );
  let frame =
    Rect::from_center_size(region.center(), region.size().max(Vec2::splat(14.0))).expand(12.0);
  let bracket = 12.0;
  for (corner, dx, dy) in [
    (frame.left_top(), 1.0, 1.0),
    (frame.right_top(), -1.0, 1.0),
    (frame.left_bottom(), 1.0, -1.0),
    (frame.right_bottom(), -1.0, -1.0),
  ] {
    for stroke in [
      Stroke::new(6.0, Color32::WHITE),
      Stroke::new(2.5, selected_color),
    ] {
      painter.line_segment([corner, corner + Vec2::new(dx * bracket, 0.0)], stroke);
      painter.line_segment([corner, corner + Vec2::new(0.0, dy * bracket)], stroke);
    }
  }
  let label = painter.layout(
    format!("Выбрано замечание {}: {}", index + 1, finding.kind.label()),
    egui::FontId::proportional(14.0),
    selected_color,
    (canvas.width() - 40.0).max(1.0),
  );
  let position = canvas.left_top() + Vec2::splat(16.0);
  painter.rect_filled(
    Rect::from_min_size(position, label.size()).expand(6.0),
    4.0,
    Color32::from_white_alpha(245),
  );
  painter.galley(position, label, selected_color);
}

fn paint_path(
  painter: &Painter,
  points: &[Point],
  closed: bool,
  screen: &impl Fn(Point) -> egui::Pos2,
  stroke: Stroke,
) {
  let mut points: Vec<_> = points.iter().copied().map(screen).collect();
  if closed && let Some(first) = points.first().copied() {
    points.push(first);
  }
  painter.add(egui::Shape::line(points, stroke));
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_multiline_finding_row_is_clickable() {
    let context = egui::Context::default();
    let finding = Finding {
      kind: IssueKind::OpenContour,
      marker: Marker::Point(Point::default()),
      detail: "Свободный конец с длинным пояснением для проверки переноса текста".repeat(3),
    };
    let mut rect = Rect::NOTHING;
    let mut output = context.run_ui(Default::default(), |ui| {
      ui.set_width(240.0);
      rect = finding_button(ui, &finding, 0, false).rect;
    });
    output.textures_delta.clear();
    assert!(rect.height() > 30.0);
    let mut clicked = false;
    for pressed in [true, false] {
      let mut output = context.run_ui(
        egui::RawInput {
          events: vec![
            egui::Event::PointerMoved(rect.center()),
            egui::Event::PointerButton {
              pos: rect.center(),
              button: egui::PointerButton::Primary,
              pressed,
              modifiers: Default::default(),
            },
          ],
          ..Default::default()
        },
        |ui| {
          ui.set_width(240.0);
          clicked |= finding_button(ui, &finding, 0, false).clicked();
        },
      );
      output.textures_delta.clear();
    }
    assert!(clicked);
  }

  #[test]
  fn selected_finding_gets_a_separate_outline_and_caption() {
    let context = egui::Context::default();
    let item = crate::diagnostics_tests::problem_areas();
    let report = crate::diagnostics::analyze(&item);
    let mut output = context.run_ui(Default::default(), |ui| {
      paint_selected_finding(
        ui.painter(),
        &item,
        &report.findings[0],
        0,
        ViewTransform {
          scale: 1.0,
          origin: egui::pos2(0.0, 1800.0),
        },
        Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 800.0)),
      );
    });
    output.textures_delta.clear();
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("Выбрано замечание 1"))));
    assert!(
      output
        .shapes
        .iter()
        .any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.stroke.width == 4.0))
    );
  }
}
