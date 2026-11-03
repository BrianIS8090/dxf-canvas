use crate::{
  cad_scene::{CadText, EntityStyle},
  geometry::{Bounds, DrawingItem, MeasureCurve, Primitive, ViewTransform},
  theme::Palette,
};
use eframe::egui::{self, Color32, Rect, Stroke};
use std::sync::Arc;

fn screen_bounds(bounds: Bounds, item: &DrawingItem, view: ViewTransform) -> Rect {
  let bounds = item.world_bounds(bounds);
  Rect::from_two_pos(
    view.world_to_screen(bounds.min),
    view.world_to_screen(bounds.max),
  )
}

pub fn paint(painter: &egui::Painter, item: &DrawingItem, view: ViewTransform) {
  let palette = Palette::get(painter.ctx());
  let clip = painter.clip_rect();
  let default_style = EntityStyle::default();
  let mut lines = crate::line_batch::LineBatch::new(painter);
  let pixels = (clip.width().max(1.0) as f64 * clip.height().max(1.0) as f64)
    * painter.ctx().pixels_per_point() as f64
    * painter.ctx().pixels_per_point() as f64;
  let mut hatch_budget = crate::hatch::PatternBudget {
    segments: (pixels / 4.0).clamp(2_000.0, 120_000.0) as usize,
    work: (pixels * 4.0).clamp(100_000.0, 4_000_000.0) as usize,
  };
  let mut hatch_overview = false;
  let tolerance = 0.2
    / (view.scale as f64 * item.scale * painter.ctx().pixels_per_point() as f64)
      .abs()
      .max(1.0e-12);
  let local_clip = Bounds::from_points([
    item.local_point(view.screen_to_world(clip.expand(3.0).left_top())),
    item.local_point(view.screen_to_world(clip.expand(3.0).right_top())),
    item.local_point(view.screen_to_world(clip.expand(3.0).right_bottom())),
    item.local_point(view.screen_to_world(clip.expand(3.0).left_bottom())),
  ]);
  let all_visible = local_clip.is_some_and(|b| {
    b.min.x <= item.bounds.min.x
      && b.max.x >= item.bounds.max.x
      && b.min.y <= item.bounds.min.y
      && b.max.y >= item.bounds.max.y
  });
  let indexed = item.appearance.render_index.matches(item.primitives.len())
    && local_clip
      .is_some_and(|b| b.width() * b.height() < item.bounds.width() * item.bounds.height() * 0.25);
  let candidates: Vec<usize> = if indexed && let Some(local_clip) = local_clip {
    item.appearance.render_index.query(local_clip)
  } else {
    (0..item.primitives.len()).collect()
  };

  let geometry = GeometryPaint {
    item,
    view,
    palette,
    tolerance,
    all_visible,
    indexed,
    default_style,
  };
  if (item.appearance.masks.is_empty()
    && item.appearance.fills.is_empty()
    && item.appearance.pattern_hatches.is_empty())
    || item.appearance.draw_steps.is_empty()
  {
    // Без перекрывающих областей сохраняется быстрый путь отрисовки линий и текста.
    for fill in &item.appearance.fills {
      paint_fill(
        painter,
        item,
        view,
        fill,
        palette.drawing_color(fill.style.color),
      );
    }
    for hatch in &item.appearance.pattern_hatches {
      hatch_overview |=
        paint_pattern_hatch(painter, item, view, hatch, &mut lines, &mut hatch_budget);
    }
    for index in candidates {
      geometry.primitive(index, &mut lines);
    }
    lines.finish();
    for text in &item.appearance.texts {
      visible_text(painter, item, view, text);
    }
    if hatch_overview {
      paint_hatch_overview_label(painter);
    }
    return;
  }
  let candidates = if indexed {
    let mut included = vec![false; item.primitives.len()];
    for index in candidates {
      included[index] = true;
    }
    Some(included)
  } else {
    None
  };
  for step in &item.appearance.draw_steps {
    match *step {
      crate::cad_scene::DrawStep::Primitive(index) => {
        if candidates.as_ref().is_none_or(|included| included[index]) {
          geometry.primitive(index, &mut lines);
        }
      }
      crate::cad_scene::DrawStep::Fill(index) => {
        lines.finish();
        let fill = &item.appearance.fills[index];
        paint_fill(
          painter,
          item,
          view,
          fill,
          palette.drawing_color(fill.style.color),
        );
      }
      crate::cad_scene::DrawStep::Text(index) => {
        lines.finish();
        visible_text(painter, item, view, &item.appearance.texts[index]);
      }
      crate::cad_scene::DrawStep::Mask(index) => {
        // Сбрасываем накопленные линии перед маской, а не только в конце кадра.
        lines.finish();
        paint_fill(
          painter,
          item,
          view,
          &item.appearance.masks[index],
          palette.canvas,
        );
      }
      crate::cad_scene::DrawStep::Hatch(index) => {
        lines.finish();
        hatch_overview |= paint_pattern_hatch(
          painter,
          item,
          view,
          &item.appearance.pattern_hatches[index],
          &mut lines,
          &mut hatch_budget,
        );
      }
    }
  }
  lines.finish();
  if hatch_overview {
    paint_hatch_overview_label(painter);
  }
}

fn paint_hatch_overview_label(painter: &egui::Painter) {
  let palette = Palette::get(painter.ctx());
  let location = painter.clip_rect().left_bottom() + egui::vec2(10.0, -10.0);
  let text = "Плотная штриховка: обзор — увеличьте масштаб";
  let galley = painter.layout_no_wrap(
    text.to_owned(),
    egui::FontId::proportional(11.0),
    palette.drawing_color(Color32::GRAY),
  );
  let origin = location - egui::vec2(0.0, galley.size().y);
  painter.rect_filled(
    Rect::from_min_size(origin, galley.size()).expand(3.0),
    3.0,
    palette.canvas,
  );
  painter.galley(origin, galley, palette.drawing_color(Color32::GRAY));
}

fn paint_pattern_hatch(
  painter: &egui::Painter,
  item: &DrawingItem,
  view: ViewTransform,
  hatch: &crate::cad_scene::CadPatternHatch,
  lines: &mut crate::line_batch::LineBatch<'_>,
  budget: &mut crate::hatch::PatternBudget,
) -> bool {
  let fill = &hatch.overview;
  if !item.appearance.visible(&fill.style)
    || !painter
      .clip_rect()
      .intersects(screen_bounds(fill.bounds, item, view))
  {
    return false;
  }
  let palette = Palette::get(painter.ctx());
  let stroke = Stroke::new(
    (fill.style.line_weight * 2.0).clamp(0.7, 2.5),
    palette.drawing_color(fill.style.color),
  );
  let scale = (item.scale * view.scale as f64).abs();
  let dpi = painter.ctx().pixels_per_point() as f64;
  let mut overview = false;
  let mut uncovered = 1.0;
  for pattern in &hatch.geometry.source.patterns {
    let (sin, cos) = pattern.angle.sin_cos();
    let tangent = hatch
      .transform
      .vector(crate::geometry::Point::new(cos, sin));
    let normal = hatch
      .transform
      .vector(crate::geometry::Point::new(-sin, cos));
    let tangent_length = tangent.x.hypot(tangent.y);
    let step = (-pattern.offset.x * sin + pattern.offset.y * cos).abs();
    let spacing =
      (tangent.x * normal.y - tangent.y * normal.x).abs() / tangent_length * step * scale;
    let period: f64 = pattern.dashes.iter().map(|d| d.abs()).sum();
    let period_pixels = period * tangent_length * scale;
    overview |= !spacing.is_finite()
      || spacing * dpi < 2.0
      || (!pattern.dashes.is_empty() && (!period_pixels.is_finite() || period_pixels * dpi < 2.0));
    let duty = if pattern.dashes.is_empty() {
      1.0
    } else {
      pattern.dashes.iter().filter(|d| **d > 0.0).sum::<f64>() / period
    };
    let dot_area = pattern.dashes.iter().filter(|d| **d == 0.0).count() as f64
      * std::f64::consts::PI
      * 1.3
      * 1.3;
    let density = stroke.width as f64 / spacing.max(1e-15) * duty
      + if period_pixels > 0.0 {
        dot_area / (period_pixels * spacing.max(1e-15))
      } else {
        0.0
      };
    uncovered *= 1.0 - density.clamp(0.0, 1.0);
  }
  if !overview {
    let exact = hatch.transform.inverse().and_then(|inverse| {
      let clip = painter.clip_rect().expand(3.0);
      let window = [
        clip.left_top(),
        clip.right_top(),
        clip.right_bottom(),
        clip.left_bottom(),
      ]
      .map(|p| inverse.apply(item.local_point(view.screen_to_world(p))));
      hatch.geometry.lines_in_view(window, budget)
    });
    if let Some(segments) = exact {
      let screen_segments: Vec<_> = segments
        .iter()
        .map(|segment| {
          segment.map(|p| view.world_to_screen(item.world_point(hatch.transform.apply(p))))
        })
        .collect();
      if screen_segments
        .iter()
        .flatten()
        .all(|p| p.x.is_finite() && p.y.is_finite())
      {
        for (segment, screen) in segments.iter().zip(screen_segments) {
          if screen[0] == screen[1] {
            lines.push(egui::Shape::circle_filled(
              screen[0],
              if segment[0] == segment[1] {
                1.3
              } else {
                stroke.width * 0.5
              },
              stroke.color,
            ));
          } else {
            lines.line(screen, stroke);
          }
        }
        return false;
      }
    }
  }
  // Обзор сохраняет островки/отверстия, цвет и исходную прозрачность; это не замена исходных данных.
  lines.finish();
  let opacity = if uncovered.is_finite() {
    (1.0 - uncovered).clamp(0.08, 1.0)
  } else {
    1.0
  };
  paint_fill(
    painter,
    item,
    view,
    fill,
    stroke.color.gamma_multiply(opacity as f32),
  );
  true
}

fn visible_text(painter: &egui::Painter, item: &DrawingItem, view: ViewTransform, text: &CadText) {
  if item.appearance.visible(&text.style)
    && painter
      .clip_rect()
      .intersects(screen_bounds(text.bounds, item, view).expand(10.0))
  {
    paint_text(painter, item, view, text);
  }
}

fn paint_fill(
  painter: &egui::Painter,
  item: &DrawingItem,
  view: ViewTransform,
  fill: &crate::cad_scene::CadFill,
  color: Color32,
) {
  if !item.appearance.visible(&fill.style)
    || !painter
      .clip_rect()
      .intersects(screen_bounds(fill.bounds, item, view))
  {
    return;
  }
  let mut mesh = egui::Mesh::default();
  mesh
    .vertices
    .extend(fill.vertices.iter().map(|point| egui::epaint::Vertex {
      pos: view.world_to_screen(item.world_point(*point)),
      uv: egui::epaint::WHITE_UV,
      color,
    }));
  mesh.indices.clone_from(&fill.indices);
  painter.add(egui::Shape::mesh(mesh));
}

struct GeometryPaint<'a> {
  item: &'a DrawingItem,
  view: ViewTransform,
  palette: Palette,
  tolerance: f64,
  all_visible: bool,
  indexed: bool,
  default_style: EntityStyle,
}

impl GeometryPaint<'_> {
  fn primitive(&self, index: usize, lines: &mut crate::line_batch::LineBatch<'_>) {
    let item = self.item;
    let view = self.view;
    let palette = self.palette;
    let tolerance = self.tolerance;
    let all_visible = self.all_visible;
    let indexed = self.indexed;
    let default_style = &self.default_style;
    let clip = lines.clip_rect();
    let primitive = &item.primitives[index];
    if !item.appearance.primitive_visible(index) {
      return;
    }
    let bounds = item
      .appearance
      .primitive_bounds
      .get(index)
      .copied()
      .or_else(|| primitive.bounds());
    if !all_visible
      && !indexed
      && bounds
        .is_some_and(|bounds| !clip.intersects(screen_bounds(bounds, item, view).expand(3.0)))
    {
      return;
    }
    let style = item.appearance.styles.get(index).unwrap_or(default_style);
    let stroke = Stroke::new(
      (style.line_weight * 2.0).clamp(0.7, 2.5),
      palette.drawing_color(style.color),
    );
    if let Primitive::Path { points, .. } = primitive
      && points.len() == 2
      && style.pattern.is_empty()
    {
      lines.line(
        [
          view.world_to_screen(item.world_point(points[0])),
          view.world_to_screen(item.world_point(points[1])),
        ],
        stroke,
      );
      return;
    }
    lines.flush();
    match primitive {
      Primitive::Point(point) => {
        lines.push(egui::Shape::circle_filled(
          view.world_to_screen(item.world_point(*point)),
          1.3,
          stroke.color,
        ));
      }
      Primitive::Path {
        points,
        closed,
        curves,
      } => {
        if style.pattern.is_empty()
          && let [MeasureCurve::Round(curve)] = curves.as_slice()
          && curve.is_full()
          && !curve.approximate
        {
          lines.push(egui::Shape::circle_stroke(
            view.world_to_screen(item.world_point(curve.center)),
            (curve.radius * item.scale * view.scale as f64) as f32,
            stroke,
          ));
          return;
        }
        let displayed = if style.pattern.is_empty() {
          item
            .appearance
            .display_geometry
            .path(index, points, tolerance)
        } else {
          points
        };
        let mut screen: Vec<_> = displayed
          .iter()
          .map(|point| view.world_to_screen(item.world_point(*point)))
          .collect();
        if *closed && screen.len() > 2 {
          screen.push(screen[0]);
        }
        // После перевода f64 в экранные f32 соседние вершины могут совпасть.
        // Три совпадения вызывают деление на ноль в соединениях пути epaint.
        screen.dedup();
        if screen.len() == 1 {
          lines.push(egui::Shape::circle_filled(
            screen[0],
            stroke.width * 0.5,
            stroke.color,
          ));
          return;
        }
        let pattern: Vec<_> = style
          .pattern
          .iter()
          .map(|length| (*length * item.scale * view.scale as f64) as f32)
          .collect();
        if pattern.len() < 2 || pattern.iter().map(|n| n.abs()).sum::<f32>() < 5.0 {
          lines.push(egui::Shape::line(screen, stroke));
        } else {
          dashed(lines, &screen, stroke, &pattern);
        }
      }
    }
  }
}

fn dashed(
  lines: &mut crate::line_batch::LineBatch<'_>,
  points: &[egui::Pos2],
  stroke: Stroke,
  pattern: &[f32],
) {
  let lengths: Vec<_> = pattern
    .iter()
    .map(|value| (value.abs() as f64).max(0.8))
    .collect();
  let period: f64 = lengths.iter().sum();
  if !period.is_finite() || period <= 0.0 {
    return;
  }
  let mut travelled = 0.0_f64;
  for pair in points.windows(2) {
    let dx = pair[1].x as f64 - pair[0].x as f64;
    let dy = pair[1].y as f64 - pair[0].y as f64;
    let length = dx.hypot(dy);
    if length < 1.0e-6 {
      continue;
    }
    let start_phase = travelled;
    travelled = (travelled + length).rem_euclid(period);
    let Some((start, end)) = clip_line(pair[0], pair[1], lines.clip_rect().expand(3.0)) else {
      continue;
    };
    let mut along = start * length;
    let finish = end * length;
    let mut phase = (start_phase + along).rem_euclid(period);
    let mut index = 0;
    while phase >= lengths[index] && index + 1 < lengths.len() {
      phase -= lengths[index];
      index += 1;
    }
    let mut remaining = lengths[index] - phase;
    while along < finish {
      let step = remaining.min(finish - along);
      let end = along + step;
      // Двойная точность и явный остаток исключают застревание на границе штриха.
      if end <= along {
        break;
      }
      if pattern[index] >= 0.0 {
        lines.push(egui::Shape::line_segment(
          [
            egui::pos2(
              (pair[0].x as f64 + dx * (along / length)) as f32,
              (pair[0].y as f64 + dy * (along / length)) as f32,
            ),
            egui::pos2(
              (pair[0].x as f64 + dx * (end / length)) as f32,
              (pair[0].y as f64 + dy * (end / length)) as f32,
            ),
          ],
          stroke,
        ));
      }
      remaining -= step;
      along = end;
      if remaining <= 1.0e-9 {
        index = (index + 1) % pattern.len();
        remaining = lengths[index];
      }
    }
  }
}

fn clip_line(a: egui::Pos2, b: egui::Pos2, rect: Rect) -> Option<(f64, f64)> {
  let mut low = 0.0_f64;
  let mut high = 1.0_f64;
  for (a, b, min, max) in [
    (a.x, b.x, rect.min.x, rect.max.x),
    (a.y, b.y, rect.min.y, rect.max.y),
  ] {
    let delta = b as f64 - a as f64;
    if delta.abs() < 1.0e-12 {
      if a < min || a > max {
        return None;
      }
    } else {
      let start = (min as f64 - a as f64) / delta;
      let end = (max as f64 - a as f64) / delta;
      low = low.max(start.min(end));
      high = high.min(start.max(end));
      if high < low {
        return None;
      }
    }
  }
  Some((low, high))
}

fn paint_text(painter: &egui::Painter, item: &DrawingItem, view: ViewTransform, text: &CadText) {
  let palette = Palette::get(painter.ctx());
  let scale = item.scale * view.scale as f64;
  let pixel_height = text.height * text.y_axis.x.hypot(text.y_axis.y) * scale;
  if pixel_height < 0.55 {
    return;
  }
  // Постоянная база исключает скачки переноса при масштабировании холста.
  let base_size = 64.0;
  let family = painter.ctx().fonts(|fonts| {
    let family = text.font.family();
    if fonts.definitions().families.contains_key(&family) {
      family
    } else {
      egui::FontFamily::Proportional
    }
  });
  let capital = painter.layout_no_wrap(
    "H".into(),
    egui::FontId::new(base_size, family.clone()),
    Color32::BLACK,
  );
  let cap_height = capital
    .rows
    .first()
    .and_then(|row| row.glyphs.first())
    .map(|glyph| glyph.uv_rect.size.y)
    .filter(|height| *height > 0.0)
    .unwrap_or(base_size);
  let factor = text.height * scale / cap_height as f64;
  let (proxy_width, tracking, slant) = if text.baseline && text.text.starts_with("\\T") {
    crate::cad_text::proxy_parameters(&text.text)
  } else {
    (1.0, 1.0, 0.0)
  };
  let wrap = if text.width > 0.0 {
    (text.width / (text.height * text.width_factor.abs().max(1.0e-6)) * cap_height as f64) as f32
  } else {
    f32::INFINITY
  };
  let mut job = crate::cad_text::layout_with_font(
    &text.text,
    base_size,
    text.style.color,
    wrap,
    text.line_spacing,
    text.height,
    text.font,
  );
  painter.ctx().fonts(|fonts| {
    for section in &mut job.sections {
      section.format.color = palette.drawing_color(section.format.color);
      section.format.underline.color = palette.drawing_color(section.format.underline.color);
      // Межбуквенный интервал резервного шрифта остаётся приближённым.
      section.format.extra_letter_spacing = (tracking as f32 - 1.0) * cap_height;
      if !fonts
        .definitions()
        .families
        .contains_key(&section.format.font_id.family)
      {
        section.format.font_id.family = family.clone();
      }
      // В DXF стандартный межстрочный интервал составляет 5/3 высоты букв.
      section.format.line_height = Some(
        cap_height * (5.0 / 3.0) * text.line_spacing * section.format.font_id.size / base_size,
      );
    }
  });
  let original = painter.layout_job(job);
  let y_anchor = if text.baseline {
    original
      .rows
      .first()
      .and_then(|row| row.glyphs.first().map(|glyph| row.pos.y + glyph.pos.y))
      .unwrap_or(0.0)
  } else if original.mesh_bounds.is_finite() {
    original.mesh_bounds.min.y + original.mesh_bounds.height() * text.alignment[1] as f32
  } else {
    original.size().y * text.alignment[1] as f32
  };
  let anchor = egui::vec2(original.size().x * text.alignment[0] as f32, y_anchor);
  let mut galley = (*original).clone();
  let mut bounds = Rect::NOTHING;
  for row in &mut galley.rows {
    let offset = row.pos.to_vec2() - anchor;
    let row_data = Arc::make_mut(&mut row.row);
    let mut row_bounds = Rect::NOTHING;
    for vertex in &mut row_data.visuals.mesh.vertices {
      let p = vertex.pos.to_vec2() + offset;
      let x = (p.x as f64 * text.width_factor * proxy_width - p.y as f64 * slant) * factor;
      let y = p.y as f64 * factor;
      let rotated = item.rotation.apply(crate::geometry::Point::new(
        x * text.x_axis.x - y * text.y_axis.x,
        x * text.x_axis.y - y * text.y_axis.y,
      ));
      vertex.pos = egui::pos2(rotated.x as f32, -rotated.y as f32);
      row_bounds.extend_with(vertex.pos);
    }
    row_data.visuals.mesh_bounds = row_bounds;
    row.pos = egui::Pos2::ZERO;
    bounds = bounds.union(row_bounds);
  }
  galley.mesh_bounds = bounds;
  galley.rect = bounds;
  painter.add(egui::epaint::TextShape::new(
    view.world_to_screen(item.world_point(text.origin)),
    Arc::new(galley),
    Color32::BLACK,
  ));
}

pub fn layers_ui(ui: &mut egui::Ui, item: &mut DrawingItem, filter: &mut String) -> bool {
  let mut changed = false;
  egui::CollapsingHeader::new(format!("Слои ({})", item.appearance.layers.len()))
    .default_open(true)
    .show(ui, |ui| {
      ui.add(
        egui::TextEdit::singleline(filter)
          .hint_text("Поиск слоя")
          .desired_width(f32::INFINITY),
      );
      ui.horizontal_wrapped(|ui| {
        if ui.small_button("Все").clicked() {
          for layer in &mut item.appearance.layers {
            layer.visible = true;
          }
          changed = true;
        }
        if ui.small_button("Ни одного").clicked() {
          for layer in &mut item.appearance.layers {
            layer.visible = false;
          }
          changed = true;
        }
        if ui.small_button("Как в DXF").clicked() {
          item.appearance.reset_layers();
          changed = true;
        }
      });
      let query = filter.to_lowercase();
      egui::ScrollArea::vertical()
        .id_salt(("layers", &item.path))
        .max_height(ui.available_height().max(80.0))
        .show(ui, |ui| {
          let mut matches = 0;
          for (index, layer) in item.appearance.layers.iter_mut().enumerate() {
            if !query.is_empty() && !layer.name.to_lowercase().contains(&query) {
              continue;
            }
            matches += 1;
            ui.push_id(index, |ui| {
              ui.horizontal(|ui| {
                let (rect, _) =
                  ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().rect_filled(
                  rect,
                  1.0,
                  Palette::get(ui.ctx()).drawing_color(layer.color),
                );
                let checkbox = ui.checkbox(&mut layer.visible, "");
                let label = ui
                  .add(egui::Label::new(&layer.name).truncate())
                  .on_hover_text(format!(
                    "{}\n{} элементов{}",
                    layer.name,
                    layer.count,
                    if layer.locked {
                      " · слой заблокирован в DXF"
                    } else {
                      ""
                    }
                  ));
                changed |= checkbox.labelled_by(label.id).changed();
              })
            });
          }
          if matches == 0 {
            ui.label("Слои не найдены. Измените запрос поиска.");
          }
        });
    });
  changed
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geometry::Point;

  fn dense_hatch_item() -> DrawingItem {
    let ring = |radius: f64| {
      vec![
        Point::new(-radius, -radius),
        Point::new(radius, -radius),
        Point::new(radius, radius),
        Point::new(-radius, radius),
      ]
    };
    let source = crate::hatch::Hatch {
      loops: vec![ring(100_000.0), ring(10.0)],
      solid: false,
      style: 0,
      patterns: vec![crate::hatch::Pattern {
        angle: 0.0,
        base: Point::default(),
        offset: Point::new(0.0, 1.0),
        dashes: vec![1.0, -1.0],
      }],
    };
    let geometry = Arc::new(source.prepare_view().unwrap());
    let (vertices, indices) = source.triangulate().unwrap();
    let bounds = Bounds::from_points(vertices.iter().copied()).unwrap();
    let style = EntityStyle {
      color: Color32::BLUE,
      ..Default::default()
    };
    let hatch = crate::cad_scene::CadPatternHatch {
      geometry,
      transform: crate::dxf_import::Transform2::IDENTITY,
      overview: crate::cad_scene::CadFill {
        vertices,
        indices,
        style,
        bounds,
      },
    };
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.primitives.clear();
    item.scale = 1.0;
    item.offset = Point::default();
    item.rotation = Default::default();
    item.bounds = bounds;
    item.appearance = crate::cad_scene::Appearance {
      layers: vec![crate::cad_scene::Layer {
        name: "Dense".to_owned(),
        color: Color32::BLUE,
        visible: true,
        initial_visible: true,
        locked: false,
        count: 1,
      }],
      pattern_hatches: vec![hatch],
      draw_steps: vec![crate::cad_scene::DrawStep::Hatch(0)],
      ..Default::default()
    };
    item
  }

  #[test]
  fn dense_hatch_switches_between_bounded_overview_and_exact_detail() {
    let mut item = dense_hatch_item();
    for (scale, hidden, expected_overview) in [
      (0.0001, false, true),
      (10.0, false, false),
      (1e-30, false, true),
      (10.0, true, false),
    ] {
      item.appearance.layers[0].visible = !hidden;
      let context = egui::Context::default();
      let mut overview = false;
      let mut output = context.run_ui(Default::default(), |ui| {
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
          egui::pos2(0.0, 0.0),
          egui::pos2(500.0, 500.0),
        ));
        let mut lines = crate::line_batch::LineBatch::new(&painter);
        let mut budget = crate::hatch::PatternBudget {
          segments: 10_000,
          work: 100_000,
        };
        overview = paint_pattern_hatch(
          &painter,
          &item,
          ViewTransform {
            scale,
            origin: egui::pos2(250.0, 250.0),
          },
          &item.appearance.pattern_hatches[0],
          &mut lines,
          &mut budget,
        );
        lines.finish();
      });
      assert_eq!(
        overview, expected_overview,
        "scale={scale}, hidden={hidden}"
      );
      output.textures_delta.clear();
      let meshes = context.tessellate(output.shapes, output.pixels_per_point);
      let vertices: Vec<_> = meshes
        .iter()
        .filter_map(|m| match &m.primitive {
          egui::epaint::Primitive::Mesh(m) => Some(&m.vertices),
          _ => None,
        })
        .flatten()
        .collect();
      assert!(vertices.len() < 150_000);
      assert!(
        vertices
          .iter()
          .all(|v| v.pos.x.is_finite() && v.pos.y.is_finite())
      );
      if hidden {
        assert!(vertices.is_empty());
      } else {
        assert!(!vertices.is_empty());
      }
    }
  }

  #[test]
  fn dense_hatch_order_respects_wipeout_and_overview_is_marked() {
    let mut item = dense_hatch_item();
    item.primitives = vec![
      Primitive::Path {
        points: vec![Point::new(-20.0, -5.0), Point::new(20.0, -5.0)],
        closed: false,
        curves: vec![],
      },
      Primitive::Path {
        points: vec![Point::new(-20.0, 5.0), Point::new(20.0, 5.0)],
        closed: false,
        curves: vec![],
      },
    ];
    item.appearance.styles = vec![
      EntityStyle {
        color: Color32::RED,
        ..Default::default()
      },
      EntityStyle {
        color: Color32::GREEN,
        ..Default::default()
      },
    ];
    item.appearance.masks = vec![crate::cad_scene::CadFill {
      vertices: vec![
        Point::new(-2.0, -2.0),
        Point::new(2.0, -2.0),
        Point::new(2.0, 2.0),
        Point::new(-2.0, 2.0),
      ],
      indices: vec![0, 1, 2, 0, 2, 3],
      style: EntityStyle::default(),
      bounds: Bounds {
        min: Point::new(-2.0, -2.0),
        max: Point::new(2.0, 2.0),
      },
    }];
    item.appearance.draw_steps = vec![
      crate::cad_scene::DrawStep::Primitive(0),
      crate::cad_scene::DrawStep::Hatch(0),
      crate::cad_scene::DrawStep::Mask(0),
      crate::cad_scene::DrawStep::Primitive(1),
    ];
    for scale in [10.0, 0.0001] {
      let context = egui::Context::default();
      crate::theme::apply(&context, false);
      let mut output = context.run_ui(Default::default(), |ui| {
        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
          egui::pos2(0.0, 0.0),
          egui::pos2(500.0, 500.0),
        ));
        paint(
          &painter,
          &item,
          ViewTransform {
            scale,
            origin: egui::pos2(250.0, 250.0),
          },
        );
      });
      output.textures_delta.clear();
      if scale == 10.0 {
        let mut order = Vec::new();
        for shape in output.shapes {
          if let egui::Shape::Mesh(mesh) = shape.shape {
            let kind = if mesh
              .vertices
              .iter()
              .all(|v| v.color == Palette::new(false).canvas)
            {
              Some("mask")
            } else if mesh.vertices.iter().any(|v| {
              v.color.r() as u16 > v.color.g() as u16 * 3 / 2
                && v.color.r() as u16 > v.color.b() as u16 * 3 / 2
            }) {
              Some("before")
            } else if mesh.vertices.iter().any(|v| {
              v.color.b() as u16 > v.color.r() as u16 * 3 / 2
                && v.color.b() as u16 > v.color.g() as u16 * 3 / 2
            }) {
              Some("hatch")
            } else if mesh.vertices.iter().any(|v| {
              v.color.g() as u16 > v.color.r() as u16 * 3 / 2
                && v.color.g() as u16 > v.color.b() as u16 * 3 / 2
            }) {
              Some("after")
            } else {
              None
            };
            if let Some(kind) = kind
              && order.last() != Some(&kind)
            {
              order.push(kind);
            }
          }
        }
        assert_eq!(order, ["before", "hatch", "mask", "after"]);
      } else {
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text.contains("увеличьте масштаб"))));
      }
    }
  }

  fn rendered_text(source: &str) -> egui::epaint::TextShape {
    let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, _) =
      crate::dxf_scene::extract(&drawing, &crate::raw_dxf::RawDxf::parse(source));
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.appearance = appearance;
    let context = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    crate::cad_text::fonts::install(&mut fonts);
    context.set_fonts(fonts);
    let mut output = context.run_ui(Default::default(), |ui| {
      paint_text(
        ui.painter(),
        &item,
        ViewTransform {
          scale: 10.0,
          origin: egui::pos2(200.0, 200.0),
        },
        &item.appearance.texts[0],
      );
    });
    output.textures_delta.clear();
    output
      .shapes
      .into_iter()
      .find_map(|shape| match shape.shape {
        egui::Shape::Text(text) => Some(text),
        _ => None,
      })
      .unwrap()
  }

  #[test]
  fn screen_collapsed_path_vertices_remain_finite_and_keep_a_visible_mark() {
    for points in [
      vec![Point::new(0.0, 0.0); 6],
      (0..6).map(|i| Point::new(i as f64 * 1e-10, 0.0)).collect(),
      vec![
        Point::new(0.0, 0.0),
        Point::new(0.0, 0.0),
        Point::new(0.0, 0.0),
        Point::new(100.0, 0.0),
      ],
    ] {
      let before = points.clone();
      let mut item =
        crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
      item.primitives = vec![Primitive::Path {
        points,
        closed: false,
        curves: vec![],
      }];
      item.appearance = Default::default();
      item.bounds = Bounds {
        min: Point::new(0.0, 0.0),
        max: Point::new(100.0, 100.0),
      };
      let context = egui::Context::default();
      let mut output = context.run_ui(Default::default(), |ui| {
        paint(
          ui.painter(),
          &item,
          ViewTransform {
            scale: 1.0,
            origin: egui::pos2(100.0, 100.0),
          },
        )
      });
      output.textures_delta.clear();
      let meshes = context.tessellate(output.shapes, output.pixels_per_point);
      let vertices: Vec<_> = meshes
        .iter()
        .filter_map(|p| match &p.primitive {
          egui::epaint::Primitive::Mesh(m) => Some(&m.vertices),
          _ => None,
        })
        .flatten()
        .collect();
      assert!(
        !vertices.is_empty(),
        "Схлопнувшийся путь должен оставаться точкой"
      );
      assert!(
        vertices
          .iter()
          .all(|v| v.pos.x.is_finite() && v.pos.y.is_finite()),
        "Совпадение экранных точек породило NaN"
      );
      let Primitive::Path { points, .. } = &item.primitives[0] else {
        panic!()
      };
      assert_eq!(points, &before, "Исходный путь изменён");
      if before.last().unwrap().x == 100.0 {
        assert!(
          vertices.iter().any(|v| v.pos.x >= 199.9),
          "Конец пути потерян"
        );
      }
    }
  }

  #[test]
  #[ignore = "Нужен локальный DXF_RENDER_FIXTURE; производственные файлы не публикуются"]
  fn private_render_fixture_has_finite_vertices() {
    let path = std::env::var("DXF_RENDER_FIXTURE").expect("Не задан DXF_RENDER_FIXTURE");
    let item = crate::dxf_import::load_dxf(std::path::Path::new(&path)).unwrap();
    let context = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    crate::cad_text::fonts::install(&mut fonts);
    context.set_fonts(fonts);
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
    let center = item.bounds.center();
    let scale = (1180.0 / item.bounds.width()).min(720.0 / item.bounds.height()) as f32;
    let view = ViewTransform {
      scale,
      origin: egui::pos2(
        640.0 - center.x as f32 * scale,
        410.0 + center.y as f32 * scale,
      ),
    };
    let mut output = context.run_ui(
      egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
      },
      |ui| paint(ui.painter(), &item, view),
    );
    output.textures_delta.clear();
    for (i, shape) in output.shapes.into_iter().enumerate() {
      let meshes = context.tessellate(vec![shape.clone()], output.pixels_per_point);
      let invalid=meshes.iter().any(|p|matches!(&p.primitive,egui::epaint::Primitive::Mesh(m) if m.vertices.iter().any(|v|!v.pos.x.is_finite()||!v.pos.y.is_finite())));
      if invalid {
        println!("INVALID shape={i} shape={:?}", shape.shape);
      }
      assert!(!invalid, "Неконечная фигура {i}");
    }
  }

  #[test]
  fn ordered_fill_covers_earlier_lines_without_a_wipeout() {
    let source = concat!(
      "0\nSECTION\n2\nENTITIES\n",
      "0\nLINE\n5\n10\n62\n1\n10\n0\n20\n5\n11\n10\n21\n5\n",
      "0\nSOLID\n5\n20\n62\n5\n10\n0\n20\n0\n11\n10\n21\n0\n12\n0\n22\n10\n13\n10\n23\n10\n",
      "0\nLINE\n5\n30\n62\n3\n10\n0\n20\n6\n11\n10\n21\n6\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (primitives, appearance, missing) =
      crate::dxf_scene::extract(&drawing, &crate::raw_dxf::RawDxf::parse(source));
    assert_eq!(missing, 0);
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.primitives = primitives;
    item.appearance = appearance;
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
      paint(
        ui.painter(),
        &item,
        ViewTransform {
          scale: 10.0,
          origin: egui::pos2(100.0, 200.0),
        },
      )
    });
    output.textures_delta.clear();
    let colors: Vec<_> = output
      .shapes
      .into_iter()
      .filter_map(|s| match s.shape {
        egui::Shape::Mesh(m) => m.vertices.iter().max_by_key(|v| v.color.a()).map(|v| {
          if v.color.b() > v.color.r() && v.color.b() > v.color.g() {
            "blue"
          } else if v.color.r() > v.color.g() {
            "red"
          } else {
            "green"
          }
        }),
        _ => None,
      })
      .collect();
    assert_eq!(colors, ["red", "blue", "green"]);
  }

  #[test]
  fn wipeout_keeps_large_line_runs_batched() {
    let mut drawing = dxf::Drawing::new();
    for i in 0..10_000 {
      if i == 5_000 {
        drawing.add_entity(dxf::entities::Entity::new(
          dxf::entities::EntityType::Wipeout(dxf::entities::Wipeout {
            location: dxf::Point::new(20.0, 20.0, 0.0),
            image_size: dxf::Vector::new(1.0, 1.0, 0.0),
            u_vector: dxf::Vector::new(40.0, 0.0, 0.0),
            v_vector: dxf::Vector::new(0.0, 40.0, 0.0),
            display_options_flags: 1,
            use_clipping: false,
            ..Default::default()
          }),
        ));
      }
      let y = (i % 100) as f64;
      drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
        dxf::entities::Line::new(dxf::Point::new(0.0, y, 0.0), dxf::Point::new(100.0, y, 0.0)),
      )));
    }
    let (primitives, appearance, missing) =
      crate::dxf_scene::extract(&drawing, &Default::default());
    assert_eq!(missing, 0);
    assert_eq!(appearance.draw_steps.len(), 10_001);
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.primitives = primitives;
    item.appearance = appearance;
    item.bounds = Bounds {
      min: Point::new(0.0, 0.0),
      max: Point::new(100.0, 100.0),
    };
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
      paint(
        ui.painter(),
        &item,
        ViewTransform {
          scale: 1.0,
          origin: egui::pos2(50.0, 150.0),
        },
      )
    });
    output.textures_delta.clear();
    assert!(
      output.shapes.len() < 30,
      "Линии перед маской и после неё должны оставаться пакетами: {}",
      output.shapes.len()
    );
  }

  #[test]
  fn wipeout_masks_only_preceding_geometry_in_both_themes() {
    let source = concat!(
      "0\nSECTION\n2\nENTITIES\n0\nLINE\n5\nA\n62\n1\n10\n0\n20\n5\n11\n10\n21\n5\n",
      "0\nTEXT\n5\nB\n10\n2\n20\n3\n40\n1\n1\nBefore\n",
      "0\nWIPEOUT\n5\nC\n8\nMask\n10\n2\n20\n2\n11\n6\n21\n0\n12\n0\n22\n6\n13\n1\n23\n1\n70\n7\n280\n1\n71\n1\n91\n2\n14\n-0.5\n24\n-0.5\n14\n0.5\n24\n0.5\n",
      "0\nLINE\n5\nD\n62\n3\n10\n0\n20\n6\n11\n10\n21\n6\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (primitives, appearance, missing) =
      crate::dxf_scene::extract(&drawing, &crate::raw_dxf::RawDxf::parse(source));
    assert_eq!(missing, 0);
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.primitives = primitives;
    item.appearance = appearance;
    for (dark, hidden) in [(false, false), (true, false), (false, true), (true, true)] {
      let mask_layer = item.appearance.masks[0].style.layer;
      item.appearance.layers[mask_layer].visible = !hidden;
      let context = egui::Context::default();
      crate::theme::apply(&context, dark);
      let mut output = context.run_ui(Default::default(), |ui| {
        paint(
          ui.painter(),
          &item,
          ViewTransform {
            scale: 10.0,
            origin: egui::pos2(100.0, 200.0),
          },
        )
      });
      output.textures_delta.clear();
      let mut order = Vec::new();
      for clipped in output.shapes {
        match clipped.shape {
          egui::Shape::Mesh(mesh) => {
            if mesh
              .vertices
              .iter()
              .all(|v| v.color == Palette::new(dark).canvas)
            {
              order.push("mask");
            } else if mesh
              .vertices
              .iter()
              .any(|v| v.color.r() as u16 > v.color.g() as u16 * 3 / 2)
            {
              order.push("before");
            } else if mesh
              .vertices
              .iter()
              .any(|v| v.color.g() as u16 > v.color.r() as u16 * 3 / 2)
            {
              order.push("after");
            }
          }
          egui::Shape::Text(_) => order.push("text"),
          _ => {}
        }
      }
      assert_eq!(
        order,
        if hidden {
          vec!["before", "text", "after"]
        } else {
          vec!["before", "text", "mask", "after"]
        }
      );
    }
  }

  #[test]
  fn theme_switch_recolors_cad_and_inline_text_without_mutating_source() {
    let source = "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n10\n21\n0\n0\nMTEXT\n5\nAB\n10\n0\n20\n10\n40\n2.5\n1\nBlack{\\C7;White}\\C5;Blue\n0\nENDSEC\n0\nEOF\n";
    let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (primitives, appearance, _) =
      crate::dxf_scene::extract(&drawing, &crate::raw_dxf::RawDxf::parse(source));
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.primitives = primitives;
    item.appearance = appearance;
    let before = format!(
      "{:?}",
      (
        &item.primitives,
        &item.appearance.texts,
        &item.appearance.styles
      )
    );
    let context = egui::Context::default();
    crate::theme::configure(&context);
    for dark in [false, true, false] {
      crate::theme::apply(&context, dark);
      let mut output = context.run_ui(Default::default(), |ui| {
        paint(
          ui.painter(),
          &item,
          ViewTransform {
            scale: 10.0,
            origin: egui::pos2(100.0, 200.0),
          },
        );
      });
      output.textures_delta.clear();
      let text = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
          egui::Shape::Text(text) => Some(text),
          _ => None,
        })
        .unwrap();
      assert_eq!(text.galley.rows.len(), 1);
      assert_eq!(text.galley.job.sections.len(), 2);
      for (index, section) in text.galley.job.sections.iter().enumerate() {
        let source_color = if index == 1 {
          Color32::BLUE
        } else {
          crate::cad_scene::EntityStyle::default().color
        };
        assert_eq!(
          section.format.color,
          Palette::new(dark).drawing_color(source_color)
        );
      }
      assert_eq!(
        before,
        format!(
          "{:?}",
          (
            &item.primitives,
            &item.appearance.texts,
            &item.appearance.styles
          )
        )
      );
    }
  }

  #[test]
  fn cad_cap_height_matches_dxf_height_and_text_keeps_baseline() {
    for style in ["Arial", "OpenSansCondensed-Light"] {
      let source = format!(
        "0\nSECTION\n2\nENTITIES\n0\nTEXT\n10\n0\n20\n0\n40\n2.5\n7\n{style}\n1\nH\n0\nENDSEC\n0\nEOF\n"
      );
      let text = rendered_text(&source);
      let bounds = text.galley.mesh_bounds;
      assert!(
        (bounds.height() - 25.0).abs() < 0.5,
        "Высота {style}: {}",
        bounds.height()
      );
      assert!(
        bounds.max.y.abs() < 0.5,
        "Базовая линия {style}: {}",
        bounds.max.y
      );
    }
  }

  #[test]
  fn mtext_preserves_explicit_paragraphs_and_positive_width_wrapping() {
    for (content, width) in [("FIRST\\PSECOND", ""), ("FIRST SECOND THIRD", "41\n12\n")] {
      let source = format!(
        "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n5\nAB\n40\n2.5\n{width}1\n{content}\n0\nENDSEC\n0\nEOF\n"
      );
      assert!(rendered_text(&source).galley.rows.len() >= 2);
    }
  }

  #[test]
  fn dimension_mtext_without_width_renders_one_row_at_any_angle() {
    for (label, angle) in [
      ("36", 0),
      ("36", 90),
      ("%%c60", 45),
      ("%%c3.3", 45),
      ("1", 90),
      ("11.5", 90),
    ] {
      let source = format!(
        "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n5\nAB\n40\n2.5\n50\n{angle}\n71\n5\n1\n{label}\n0\nENDSEC\n0\nEOF\n"
      );
      let drawing = dxf::Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
      let (_, appearance, _) =
        crate::dxf_scene::extract(&drawing, &crate::raw_dxf::RawDxf::parse(&source));
      let mut item =
        crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
      item.appearance = appearance;
      let context = egui::Context::default();
      let mut output = context.run_ui(Default::default(), |ui| {
        paint_text(
          ui.painter(),
          &item,
          ViewTransform {
            scale: 10.0,
            origin: egui::pos2(200.0, 200.0),
          },
          &item.appearance.texts[0],
        );
      });
      output.textures_delta.clear();
      let galley = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
          egui::Shape::Text(text) => Some(&text.galley),
          _ => None,
        })
        .unwrap();
      assert_eq!(galley.rows.len(), 1, "Надпись {label}, угол {angle}");
    }
  }

  #[test]
  fn rotated_bounds_include_all_corners_not_only_the_diagonal() {
    let mut item =
      crate::dxf_import::load_dxf(std::path::Path::new("examples/advanced_demo.dxf")).unwrap();
    item.rotation = crate::geometry::Rotation::new(std::f64::consts::FRAC_PI_4);
    let view = ViewTransform {
      scale: 1.0,
      origin: egui::pos2(200.0, 200.0),
    };
    let bounds = Bounds {
      min: Point::new(0.0, 0.0),
      max: Point::new(100.0, 100.0),
    };
    let rect = screen_bounds(bounds, &item, view);
    assert!((rect.width() - 100.0 * 2.0_f32.sqrt()).abs() < 0.001);
    assert!((rect.height() - rect.width()).abs() < 0.001);
    for corner in bounds.corners() {
      assert!(
        rect
          .expand(0.001)
          .contains(view.world_to_screen(item.world_point(corner)))
      );
    }
  }

  fn dashed(painter: &egui::Painter, points: &[egui::Pos2], stroke: Stroke, pattern: &[f32]) {
    let mut lines = crate::line_batch::LineBatch::new(painter);
    super::dashed(&mut lines, points, stroke, pattern);
    lines.finish();
  }

  #[test]
  fn fractional_dash_lengths_do_not_repeat_zero_length_strokes() {
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
      dashed(
        ui.painter(),
        &[egui::pos2(0.0, 0.0), egui::pos2(5000.0, 0.0)],
        Stroke::new(0.7, Color32::BLACK),
        &[3.7139, -1.2817],
      );
    });
    output.textures_delta.clear();
    assert!(
      output.shapes.len() < 1100,
      "Повторные штрихи: {}",
      output.shapes.len()
    );
    let end = output
      .shapes
      .iter()
      .filter_map(|shape| match &shape.shape {
        egui::Shape::LineSegment { points, .. } => Some(points[1].x),
        _ => None,
      })
      .max_by(f32::total_cmp)
      .unwrap();
    assert!(end > 4990.0, "Штриховая линия оборвалась на {end}");
  }

  #[test]
  fn long_dashed_line_skips_invisible_part_without_losing_its_phase() {
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
      let painter = ui.painter().with_clip_rect(Rect::from_min_max(
        egui::pos2(0.0, -10.0),
        egui::pos2(200.0, 10.0),
      ));
      dashed(
        &painter,
        &[egui::pos2(-1_000_000.0, 0.0), egui::pos2(1_000_000.0, 0.0)],
        Stroke::new(0.7, Color32::BLACK),
        &[3.0, -3.0],
      );
    });
    output.textures_delta.clear();
    let segments: Vec<_> = output
      .shapes
      .iter()
      .filter_map(|shape| match &shape.shape {
        egui::Shape::LineSegment { points, .. } => Some(*points),
        _ => None,
      })
      .collect();
    assert!(segments.len() < 40);
    assert!(segments.last().unwrap()[1].x > 195.0);
    let visible = segments.iter().find(|points| points[0].x >= 0.0).unwrap();
    assert!((visible[0].x - 2.0).abs() < 0.001);
    assert!((visible[1].x - 5.0).abs() < 0.001);
  }

  #[test]
  fn small_circles_do_not_generate_hundreds_of_vertices_per_hole() {
    let mut drawing = dxf::Drawing::new();
    for i in 0..200 {
      drawing.add_entity(dxf::entities::Entity::new(
        dxf::entities::EntityType::Circle(dxf::entities::Circle {
          center: dxf::Point::new((i % 20) as f64 * 15.0, (i / 20) as f64 * 15.0, 0.0),
          radius: 3.0,
          ..Default::default()
        }),
      ));
    }
    let (primitives, appearance, unsupported_entities) =
      crate::dxf_scene::extract(&drawing, &Default::default());
    let item = DrawingItem {
      rotation: Default::default(),
      appearance,
      primitives,
      unsupported_entities,
      units: Default::default(),
      path: "holes.dxf".into(),
      name: "Отверстия".into(),
      bounds: Bounds {
        min: Point::new(-3.0, -3.0),
        max: Point::new(300.0, 150.0),
      },
      offset: Point::default(),
      scale: 1.0,
    };
    let context = egui::Context::default();
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 300.0));
    let mut output = context.run_ui(
      egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
      },
      |ui| {
        paint(
          ui.painter(),
          &item,
          ViewTransform {
            scale: 1.0,
            origin: egui::pos2(20.0, 200.0),
          },
        );
      },
    );
    output.textures_delta.clear();
    let rendered = context.tessellate(output.shapes, output.pixels_per_point);
    let vertices: usize = rendered
      .iter()
      .map(|shape| match &shape.primitive {
        egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
        _ => 0,
      })
      .sum();
    assert!(
      vertices < 20_000,
      "Слишком тяжёлая отрисовка мелких отверстий: {vertices} вершин"
    );
    assert_eq!(item.primitives.len(), 200);
    let Primitive::Path { curves, .. } = &item.primitives[0] else {
      panic!()
    };
    let crate::geometry::MeasureCurve::Round(curve) = curves[0] else {
      panic!()
    };
    assert_eq!(curve.radius, 3.0);
  }
}
