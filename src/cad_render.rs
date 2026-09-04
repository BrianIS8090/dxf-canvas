use crate::{
  cad_scene::{CadText, EntityStyle, readable_color},
  geometry::{Bounds, DrawingItem, MeasureCurve, Primitive, ViewTransform},
};
use eframe::egui::{self, Color32, Rect, Stroke};
use std::sync::Arc;

fn screen_bounds(bounds: Bounds, item: &DrawingItem, view: ViewTransform) -> Rect {
  Rect::from_two_pos(
    view.world_to_screen(item.world_point(bounds.min)),
    view.world_to_screen(item.world_point(bounds.max)),
  )
}

pub fn paint(painter: &egui::Painter, item: &DrawingItem, view: ViewTransform) {
  let clip = painter.clip_rect();
  for fill in &item.appearance.fills {
    if !item.appearance.visible(&fill.style)
      || !clip.intersects(screen_bounds(fill.bounds, item, view))
    {
      continue;
    }
    let mut mesh = egui::Mesh::default();
    let color = readable_color(fill.style.color);
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
  let default_style = EntityStyle::default();
  let mut lines = crate::line_batch::LineBatch::new(painter);
  let tolerance = 0.2
    / (view.scale as f64 * item.scale * painter.ctx().pixels_per_point() as f64)
      .abs()
      .max(1.0e-12);
  let local_clip = Bounds::from_points([
    item.local_point(view.screen_to_world(clip.expand(3.0).left_top())),
    item.local_point(view.screen_to_world(clip.expand(3.0).right_bottom())),
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
  for index in candidates {
    let primitive = &item.primitives[index];
    if !item.appearance.primitive_visible(index) {
      continue;
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
      continue;
    }
    let style = item.appearance.styles.get(index).unwrap_or(&default_style);
    let stroke = Stroke::new(
      (style.line_weight * 2.0).clamp(0.7, 2.5),
      readable_color(style.color),
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
      continue;
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
          continue;
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
        let pattern: Vec<_> = style
          .pattern
          .iter()
          .map(|length| (*length * item.scale * view.scale as f64) as f32)
          .collect();
        if pattern.len() < 2 || pattern.iter().map(|n| n.abs()).sum::<f32>() < 5.0 {
          lines.push(egui::Shape::line(screen, stroke));
        } else {
          dashed(&mut lines, &screen, stroke, &pattern);
        }
      }
    }
  }
  lines.finish();
  for text in &item.appearance.texts {
    if item.appearance.visible(&text.style)
      && clip.intersects(screen_bounds(text.bounds, item, view).expand(10.0))
    {
      paint_text(painter, item, view, text);
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
  let scale = item.scale * view.scale as f64;
  let pixel_height = text.height * text.y_axis.x.hypot(text.y_axis.y) * scale;
  if pixel_height < 0.55 {
    return;
  }
  let base_size = pixel_height.clamp(8.0, 64.0) as f32;
  let factor = text.height * 1.3 * scale / base_size as f64;
  let wrap = if text.width > 0.0 {
    (text.width / (text.height * 1.3) * base_size as f64) as f32
  } else {
    f32::INFINITY
  };
  let job = crate::cad_text::layout(
    &text.text,
    base_size,
    readable_color(text.style.color),
    wrap,
    text.line_spacing,
    text.height,
  );
  let original = painter.layout_job(job);
  let anchor = egui::vec2(
    original.size().x * text.alignment[0] as f32,
    original.size().y * text.alignment[1] as f32,
  );
  let mut galley = (*original).clone();
  let mut bounds = Rect::NOTHING;
  for row in &mut galley.rows {
    let offset = row.pos.to_vec2() - anchor;
    let row_data = Arc::make_mut(&mut row.row);
    let mut row_bounds = Rect::NOTHING;
    for vertex in &mut row_data.visuals.mesh.vertices {
      let p = vertex.pos.to_vec2() + offset;
      let x = p.x as f64 * factor * text.width_factor;
      let y = p.y as f64 * factor;
      vertex.pos = egui::pos2(
        (x * text.x_axis.x - y * text.y_axis.x) as f32,
        (-x * text.x_axis.y + y * text.y_axis.y) as f32,
      );
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
      ui.horizontal(|ui| {
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
        .max_height(260.0)
        .show(ui, |ui| {
          for (index, layer) in item.appearance.layers.iter_mut().enumerate() {
            if !query.is_empty() && !layer.name.to_lowercase().contains(&query) {
              continue;
            }
            ui.push_id(index, |ui| {
              ui.horizontal(|ui| {
                let (rect, _) =
                  ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter()
                  .rect_filled(rect, 1.0, readable_color(layer.color));
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
        });
    });
  changed
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geometry::Point;

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
