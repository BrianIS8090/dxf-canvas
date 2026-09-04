use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::{
  geometry::{DrawingItem, Point, RoundCurve, ViewTransform},
  measurement::{Dimension, DimensionKind, Snap},
};

pub fn paint_dimension(
  painter: &Painter,
  dimension: &Dimension,
  item: &DrawingItem,
  transform: ViewTransform,
  font_size: f32,
  preview: bool,
) {
  let color = if preview {
    Color32::from_rgb(23, 137, 111)
  } else {
    Color32::from_rgb(164, 73, 22)
  };
  let stroke = Stroke::new(1.3, color);
  let screen = |point: Point| transform.world_to_screen(item.world_point(point));
  let label_position = match dimension.kind {
    DimensionKind::Linear { start, end } => {
      let length = (end.x - start.x).hypot(end.y - start.y);
      if length < 1.0e-9 {
        return;
      }
      let normal = Point::new(-(end.y - start.y) / length, (end.x - start.x) / length);
      let offset =
        (dimension.label.x - start.x) * normal.x + (dimension.label.y - start.y) * normal.y;
      let a = screen(start);
      let b = screen(end);
      let da = screen(Point::new(
        start.x + normal.x * offset,
        start.y + normal.y * offset,
      ));
      let db = screen(Point::new(
        end.x + normal.x * offset,
        end.y + normal.y * offset,
      ));
      let direction = (db - da).normalized();
      let outside = da.distance(db) < 65.0;
      for (source, target) in [(a, da), (b, db)] {
        let extension = (target - source).normalized();
        painter.line_segment([source + extension * 3.0, target + extension * 6.0], stroke);
        painter.circle_filled(source, 2.0, color);
      }
      let extra = if outside { 14.0 } else { 0.0 };
      painter.line_segment([da - direction * extra, db + direction * extra], stroke);
      let arrow_sign = if outside { -1.0 } else { 1.0 };
      arrow(painter, da, direction * arrow_sign, color);
      arrow(painter, db, -direction * arrow_sign, color);
      da.lerp(db, 0.5) + Vec2::new(0.0, -font_size * 0.8 - 4.0)
    }
    DimensionKind::Round { curve, diameter } => {
      let center = screen(curve.center);
      let label = screen(dimension.label);
      center_mark(painter, center, color);
      if diameter {
        let direction = unit(label - center);
        let radius = (curve.radius * item.scale * transform.scale as f64) as f32;
        let a = center - direction * radius;
        let b = center + direction * radius;
        let outside = radius * 2.0 < 65.0;
        let extra = if outside { 14.0 } else { 0.0 };
        painter.line_segment([a - direction * extra, b + direction * extra], stroke);
        painter.line_segment([b, label], stroke);
        let sign = if outside { -1.0 } else { 1.0 };
        arrow(painter, a, direction * sign, color);
        arrow(painter, b, -direction * sign, color);
      } else {
        let edge = screen(curve.nearest(dimension.label));
        painter.line_segment([center, edge], stroke);
        painter.line_segment([edge, label], stroke);
        let base_direction = if label.distance(center) > edge.distance(center) {
          unit(label - edge)
        } else {
          unit(center - edge)
        };
        arrow(painter, edge, base_direction, color);
      }
      label
    }
    DimensionKind::Angle {
      first,
      vertex,
      last,
    } => {
      let center = screen(vertex);
      let radius = center.distance(screen(dimension.label)).max(24.0);
      let start = (first.y - vertex.y).atan2(first.x - vertex.x);
      let sweep = crate::measurement::angle_radians(first, vertex, last);
      let points: Vec<_> = (0..=64)
        .map(|i| {
          let angle = start + sweep * i as f64 / 64.0;
          center + Vec2::new(angle.cos() as f32, -angle.sin() as f32) * radius
        })
        .collect();
      for (source, endpoint) in [(screen(first), points[0]), (screen(last), points[64])] {
        painter.line_segment([center, source], stroke);
        painter.line_segment([source, endpoint], stroke);
      }
      arrow(painter, points[0], unit(points[1] - points[0]), color);
      arrow(painter, points[64], unit(points[63] - points[64]), color);
      let middle = points[32];
      painter.add(egui::Shape::line(points, stroke));
      center_mark(painter, center, color);
      middle + unit(middle - center) * (font_size + 8.0)
    }
    DimensionKind::Region(ref region) => {
      if preview {
        for boundary in &region.boundaries {
          painter.add(egui::Shape::line(
            boundary.iter().copied().map(&screen).collect(),
            Stroke::new(2.5, color),
          ));
        }
      }
      screen(dimension.label)
    }
  };
  paint_label(
    painter,
    label_position,
    dimension.text(item),
    font_size,
    color,
  );
}

fn arrow(painter: &Painter, tip: Pos2, base_direction: Vec2, color: Color32) {
  let perpendicular = Vec2::new(-base_direction.y, base_direction.x);
  painter.add(egui::Shape::convex_polygon(
    vec![
      tip,
      tip + base_direction * 8.0 + perpendicular * 3.0,
      tip + base_direction * 8.0 - perpendicular * 3.0,
    ],
    color,
    Stroke::NONE,
  ));
}

fn unit(vector: Vec2) -> Vec2 {
  if vector.length_sq() < 1.0e-12 {
    Vec2::X
  } else {
    vector.normalized()
  }
}

fn paint_label(painter: &Painter, center: Pos2, text: String, font_size: f32, color: Color32) {
  let galley = painter.layout_no_wrap(text, FontId::proportional(font_size), color);
  let position = center - galley.size() * 0.5;
  painter.rect_filled(
    Rect::from_min_size(position, galley.size()).expand2(Vec2::new(5.0, 3.0)),
    3.0,
    Color32::WHITE,
  );
  painter.galley(position, galley, color);
}

fn center_mark(painter: &Painter, point: Pos2, color: Color32) {
  let stroke = Stroke::new(1.0, color);
  painter.line_segment(
    [point - Vec2::new(5.0, 0.0), point + Vec2::new(5.0, 0.0)],
    stroke,
  );
  painter.line_segment(
    [point - Vec2::new(0.0, 5.0), point + Vec2::new(0.0, 5.0)],
    stroke,
  );
}

pub fn paint_snap(painter: &Painter, snap: Snap, item: &DrawingItem, transform: ViewTransform) {
  let position = transform.world_to_screen(item.world_point(snap.point));
  let color = Color32::from_rgb(13, 151, 107);
  painter.rect_stroke(
    Rect::from_center_size(position, Vec2::splat(10.0)),
    1.0,
    Stroke::new(2.0, color),
    StrokeKind::Middle,
  );
  painter.text(
    position + Vec2::new(10.0, 12.0),
    egui::Align2::LEFT_TOP,
    format!(
      "{}{}",
      snap.kind.label(),
      if snap.approximate { " ≈" } else { "" }
    ),
    FontId::proportional(12.0),
    color,
  );
}

pub fn paint_round_highlight(
  painter: &Painter,
  curve: RoundCurve,
  item: &DrawingItem,
  transform: ViewTransform,
) {
  let points = (0..=96)
    .map(|index| {
      transform.world_to_screen(
        item.world_point(curve.point_at(curve.start + curve.sweep * index as f64 / 96.0)),
      )
    })
    .collect();
  painter.add(egui::Shape::line(
    points,
    Stroke::new(2.5, Color32::from_rgb(13, 151, 107)),
  ));
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geometry::{Bounds, DrawingItem, LengthUnit, Point, ViewTransform};
  use crate::measurement::Dimension;
  use eframe::egui;

  #[test]
  fn rendered_dimension_keeps_source_value_when_detail_is_enlarged() {
    let item = DrawingItem {
      appearance: Default::default(),
      path: "detail.dxf".into(),
      name: "detail".into(),
      primitives: vec![],
      bounds: Bounds {
        min: Point::default(),
        max: Point::new(100.0, 100.0),
      },
      offset: Point::new(200.0, 0.0),
      scale: 2.0,
      unsupported_entities: 0,
      units: LengthUnit::from_dxf_code(4),
    };
    let dimension = Dimension::linear(
      0,
      Point::default(),
      Point::new(30.0, 40.0),
      Point::new(50.0, 60.0),
    );
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
      paint_dimension(
        ui.painter(),
        &dimension,
        &item,
        ViewTransform {
          scale: 2.0,
          origin: egui::pos2(0.0, 400.0),
        },
        16.0,
        false,
      );
    });
    output.textures_delta.clear();
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
      egui::Shape::Text(text) if text.galley.job.text == "50 мм")));
  }
}
