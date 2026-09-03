use dxf::{
  Drawing, Point,
  entities::{Arc, Circle, Entity, EntityType, Line, Spline},
  enums::{AcadVersion, Units},
};

pub fn diagnostics_drawing() -> Drawing {
  let mut drawing = Drawing::new();
  drawing.header.version = AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;

  // Синтетическая сетка отверстий: реальные производственные чертежи не публикуются.
  for index in 0..60 {
    circle(
      &mut drawing,
      ((index % 10) as f64 * 90.0, (index / 10) as f64 * 20.0),
      3.0,
    );
  }

  // Индексы 60–67: восемь необъединённых частей прямоугольника 128 × 52, R5.
  for (start, end) in [
    ((105.0, 200.0), (223.0, 200.0)),
    ((228.0, 205.0), (228.0, 247.0)),
    ((223.0, 252.0), (105.0, 252.0)),
    ((100.0, 247.0), (100.0, 205.0)),
  ] {
    line(&mut drawing, start, end);
  }
  for (center, start) in [
    ((223.0, 205.0), 270.0),
    ((223.0, 247.0), 0.0),
    ((105.0, 247.0), 90.0),
    ((105.0, 205.0), 180.0),
  ] {
    arc(&mut drawing, center, 5.0, start, start + 90.0);
  }

  // Индексы 68–72: окружность и четыре точных дубликата.
  for _ in 0..5 {
    circle(&mut drawing, (320.0, 220.0), 20.0);
  }
  deformed_spline(&mut drawing, (400.0, 220.0), 8.0, 0.035);
  deformed_spline(&mut drawing, (450.0, 220.0), 4.0, 0.04);
  // Индексы 75–76: две отдельные полуокружности с совпадающими концами.
  arc(&mut drawing, (510.0, 220.0), 7.5, 0.0, 180.0);
  arc(&mut drawing, (510.0, 220.0), 7.5, 180.0, 360.0);
  line(&mut drawing, (600.0, 220.0), (600.05, 220.0));
  line(&mut drawing, (650.0, 220.0), (680.0, 220.0));
  drawing
}

fn line(drawing: &mut Drawing, start: (f64, f64), end: (f64, f64)) {
  drawing.add_entity(Entity::new(EntityType::Line(Line::new(
    Point::new(start.0, start.1, 0.0),
    Point::new(end.0, end.1, 0.0),
  ))));
}

fn circle(drawing: &mut Drawing, center: (f64, f64), radius: f64) {
  drawing.add_entity(Entity::new(EntityType::Circle(Circle {
    center: Point::new(center.0, center.1, 0.0),
    radius,
    ..Default::default()
  })));
}

fn arc(drawing: &mut Drawing, center: (f64, f64), radius: f64, start: f64, end: f64) {
  drawing.add_entity(Entity::new(EntityType::Arc(Arc {
    center: Point::new(center.0, center.1, 0.0),
    radius,
    start_angle: start,
    end_angle: end,
    ..Default::default()
  })));
}

fn deformed_spline(drawing: &mut Drawing, center: (f64, f64), radius: f64, distortion: f64) {
  let count = 128;
  let mut spline = Spline {
    degree_of_curve: 1,
    control_points: (0..=count)
      .map(|i| {
        let angle = std::f64::consts::TAU * i as f64 / count as f64;
        let r = radius * (1.0 + distortion * (3.0 * angle).cos());
        Point::new(center.0 + r * angle.cos(), center.1 + r * angle.sin(), 0.0)
      })
      .collect(),
    knot_values: std::iter::once(0.0)
      .chain((0..=count).map(|i| i as f64))
      .chain(std::iter::once(count as f64))
      .collect(),
    ..Default::default()
  };
  spline.set_is_closed(true);
  drawing.add_entity(Entity::new(EntityType::Spline(spline)));
}
