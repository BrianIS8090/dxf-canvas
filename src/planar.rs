use crate::{
  geometry::{Bounds, DrawingItem, MeasureCurve, Point, Primitive, RoundCurve},
  spatial::SpatialIndex,
};
use std::f64::consts::TAU;

const MAX_EDGES: usize = 200_000;
const MAX_PAIRS: usize = 2_000_000;
const MAX_CONTACTS: usize = 10_000;

#[derive(Clone, Copy, Debug)]
pub enum EdgeShape {
  Line(Point, Point),
  Arc(RoundCurve),
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
  pub shape: EdgeShape,
  pub primitive: usize,
  pub approximate: bool,
}

impl EdgeShape {
  pub fn ends(self) -> (Point, Point) {
    match self {
      Self::Line(a, b) => (a, b),
      Self::Arc(a) => (a.point_at(a.start), a.point_at(a.start + a.sweep)),
    }
  }

  pub fn bounds(self) -> Bounds {
    let (a, b) = self.ends();
    let mut bounds = Bounds::from_points([a, b]).unwrap_or_else(Bounds::empty);
    if let Self::Arc(arc) = self {
      for i in 0..4 {
        let angle = i as f64 * TAU / 4.0;
        if arc.contains_angle(angle) {
          bounds.include(arc.point_at(angle));
        }
      }
    }
    bounds
  }

  pub fn length(self) -> f64 {
    match self {
      Self::Line(a, b) => distance(a, b),
      Self::Arc(a) => a.radius * a.sweep.abs(),
    }
  }

  pub fn reversed(self) -> Self {
    match self {
      Self::Line(a, b) => Self::Line(b, a),
      Self::Arc(a) => Self::Arc(RoundCurve {
        start: a.start + a.sweep,
        sweep: -a.sweep,
        ..a
      }),
    }
  }

  pub fn area_integral(self, origin: Point) -> f64 {
    // Интеграл Грина: прямые и круговые дуги не заменяются экранной ломаной.
    match self {
      Self::Line(a, b) => cross(sub(a, origin), sub(b, origin)) * 0.5,
      Self::Arc(a) => {
        let c = sub(a.center, origin);
        let end = a.start + a.sweep;
        (a.radius * c.x * (end.sin() - a.start.sin())
          + a.radius * c.y * (a.start.cos() - end.cos())
          + a.radius * a.radius * a.sweep)
          * 0.5
      }
    }
  }

  pub fn points(self) -> Vec<Point> {
    match self {
      Self::Line(a, b) => vec![a, b],
      Self::Arc(a) => {
        let n = (a.sweep.abs() / TAU * 512.0).ceil().max(2.0) as usize;
        (0..=n)
          .map(|i| a.point_at(a.start + a.sweep * i as f64 / n as f64))
          .collect()
      }
    }
  }

  fn is_end(self, p: Point, tolerance: f64) -> bool {
    if matches!(self, Self::Arc(a) if a.is_full()) {
      return false;
    }
    let (a, b) = self.ends();
    distance(p, a) <= tolerance || distance(p, b) <= tolerance
  }
}

#[derive(Default)]
pub struct Edges {
  pub values: Vec<Edge>,
  pub limited: bool,
}

pub fn drawing_edges(item: &DrawingItem) -> Edges {
  let mut result = Edges::default();
  for (primitive, shape) in item.primitives.iter().enumerate() {
    if !item.appearance.primitive_diagnostic(primitive) {
      continue;
    }
    let Primitive::Path { curves, .. } = shape else {
      continue;
    };
    for curve in curves {
      let mut push = |shape: EdgeShape, approximate| {
        if result.values.len() >= MAX_EDGES {
          result.limited = true;
        } else if shape.bounds().is_valid()
          && shape.length().is_finite()
          && shape.length() > 1.0e-12
        {
          result.values.push(Edge {
            shape,
            primitive,
            approximate,
          });
        }
      };
      match curve {
        MeasureCurve::Line { start, end } => push(EdgeShape::Line(*start, *end), false),
        MeasureCurve::Round(arc) => push(EdgeShape::Arc(*arc), arc.approximate),
        MeasureCurve::Polyline { points, closed } => {
          for pair in points.windows(2) {
            push(EdgeShape::Line(pair[0], pair[1]), true);
          }
          if *closed && let (Some(a), Some(b)) = (points.last(), points.first()) {
            push(EdgeShape::Line(*a, *b), true);
          }
        }
      }
      if result.limited {
        return result;
      }
    }
  }
  result
}

#[derive(Clone, Debug)]
pub enum ContactKind {
  Crossing(Point),
  Overlap { shape: EdgeShape, duplicate: bool },
}

#[derive(Clone, Debug)]
pub struct Contact {
  pub a: usize,
  pub b: usize,
  pub kind: ContactKind,
}

#[derive(Default)]
pub struct Contacts {
  pub values: Vec<Contact>,
  pub limited: bool,
}

pub fn contacts(edges: &[Edge], tolerance: f64) -> Contacts {
  let index = SpatialIndex::new(
    edges.len(),
    edges
      .iter()
      .enumerate()
      .map(|(i, e)| (i, expanded(e.shape.bounds(), tolerance))),
  );
  let mut result = Contacts::default();
  let mut pairs = 0;
  for (a, edge) in edges.iter().enumerate() {
    for b in index
      .query(expanded(edge.shape.bounds(), tolerance))
      .into_iter()
      .filter(|b| *b > a)
    {
      pairs += 1;
      if pairs > MAX_PAIRS || result.values.len() >= MAX_CONTACTS {
        result.limited = true;
        return result;
      }
      for kind in pair_contacts(edge.shape, edges[b].shape, tolerance) {
        if let ContactKind::Crossing(p) = kind
          && edge.shape.is_end(p, tolerance)
          && edges[b].shape.is_end(p, tolerance)
        {
          continue;
        }
        result.values.push(Contact { a, b, kind });
      }
    }
  }
  result
}

fn pair_contacts(a: EdgeShape, b: EdgeShape, tolerance: f64) -> Vec<ContactKind> {
  match (a, b) {
    (EdgeShape::Line(a, b), EdgeShape::Line(c, d)) => line_line(a, b, c, d, tolerance),
    (EdgeShape::Line(a, b), EdgeShape::Arc(arc)) | (EdgeShape::Arc(arc), EdgeShape::Line(a, b)) => {
      line_arc(a, b, arc, tolerance)
    }
    (EdgeShape::Arc(a), EdgeShape::Arc(b)) => arc_arc(a, b, tolerance),
  }
}

fn line_line(a: Point, b: Point, c: Point, d: Point, tolerance: f64) -> Vec<ContactKind> {
  let r = sub(b, a);
  let s = sub(d, c);
  let lr = r.x.hypot(r.y);
  let ls = s.x.hypot(s.y);
  let denominator = cross(r, s);
  if denominator.abs() <= 1.0e-12 * lr * ls {
    if cross(sub(c, a), r).abs() / lr > tolerance {
      return vec![];
    }
    let t0 = dot(sub(c, a), r) / (lr * lr);
    let t1 = dot(sub(d, a), r) / (lr * lr);
    let lo = t0.min(t1).max(0.0);
    let hi = t0.max(t1).min(1.0);
    if (hi - lo) * lr <= tolerance {
      return vec![];
    }
    let duplicate = (distance(a, c) <= tolerance && distance(b, d) <= tolerance)
      || (distance(a, d) <= tolerance && distance(b, c) <= tolerance);
    return vec![ContactKind::Overlap {
      shape: EdgeShape::Line(add(a, mul(r, lo)), add(a, mul(r, hi))),
      duplicate,
    }];
  }
  let t = cross(sub(c, a), s) / denominator;
  let u = cross(sub(c, a), r) / denominator;
  if t >= -tolerance / lr
    && t <= 1.0 + tolerance / lr
    && u >= -tolerance / ls
    && u <= 1.0 + tolerance / ls
  {
    vec![ContactKind::Crossing(add(a, mul(r, t.clamp(0.0, 1.0))))]
  } else {
    vec![]
  }
}

fn line_arc(a: Point, b: Point, arc: RoundCurve, tolerance: f64) -> Vec<ContactKind> {
  let direction = sub(b, a);
  let length = direction.x.hypot(direction.y);
  let unit = mul(direction, 1.0 / length);
  let relative = sub(arc.center, a);
  let along = dot(relative, unit);
  let height = cross(relative, unit).abs();
  if height > arc.radius + tolerance {
    return vec![];
  }
  let offset = ((arc.radius - height) * (arc.radius + height))
    .max(0.0)
    .sqrt();
  let mut points = Vec::new();
  for t in [along - offset, along + offset] {
    let p = add(a, mul(unit, t));
    if t >= -tolerance
      && t <= length + tolerance
      && on_arc(arc, p, tolerance)
      && !points.iter().any(|q| distance(*q, p) <= tolerance)
    {
      points.push(p);
    }
  }
  points.into_iter().map(ContactKind::Crossing).collect()
}

fn arc_arc(a: RoundCurve, b: RoundCurve, tolerance: f64) -> Vec<ContactKind> {
  let delta = sub(b.center, a.center);
  let d = delta.x.hypot(delta.y);
  if d <= tolerance && (a.radius - b.radius).abs() <= tolerance {
    let angular = tolerance / a.radius.max(tolerance);
    let mut result = Vec::new();
    let duplicate = (a.sweep.abs() - b.sweep.abs()).abs() <= angular
      && (a.is_full()
        || (distance(
          a.point_at(a.start + a.sweep * 0.5),
          b.point_at(b.start + b.sweep * 0.5),
        ) <= tolerance));
    for (a0, a1) in intervals(a) {
      for (b0, b1) in intervals(b) {
        let lo = a0.max(b0);
        let hi = a1.min(b1);
        if hi - lo > angular {
          result.push(ContactKind::Overlap {
            shape: EdgeShape::Arc(RoundCurve {
              start: lo,
              sweep: hi - lo,
              ..a
            }),
            duplicate,
          });
        }
      }
    }
    return result;
  }
  if d <= 1.0e-12
    || d > a.radius + b.radius + tolerance
    || d < (a.radius - b.radius).abs() - tolerance
  {
    return vec![];
  }
  let along = (a.radius * a.radius - b.radius * b.radius + d * d) / (2.0 * d);
  let height = (a.radius * a.radius - along * along).max(0.0).sqrt();
  let unit = mul(delta, 1.0 / d);
  let center = add(a.center, mul(unit, along));
  let perpendicular = Point::new(-unit.y, unit.x);
  let mut points = Vec::new();
  for sign in [-1.0, 1.0] {
    let p = add(center, mul(perpendicular, sign * height));
    if on_arc(a, p, tolerance)
      && on_arc(b, p, tolerance)
      && !points.iter().any(|q| distance(*q, p) <= tolerance)
    {
      points.push(p);
    }
  }
  points.into_iter().map(ContactKind::Crossing).collect()
}

fn on_arc(a: RoundCurve, p: Point, tolerance: f64) -> bool {
  a.contains_angle((p.y - a.center.y).atan2(p.x - a.center.x))
    || distance(a.point_at(a.start), p) <= tolerance
    || distance(a.point_at(a.start + a.sweep), p) <= tolerance
}

fn intervals(a: RoundCurve) -> Vec<(f64, f64)> {
  if a.is_full() {
    return vec![(0.0, TAU)];
  }
  let start = (if a.sweep < 0.0 {
    a.start + a.sweep
  } else {
    a.start
  })
  .rem_euclid(TAU);
  let end = start + a.sweep.abs();
  if end <= TAU {
    vec![(start, end)]
  } else {
    vec![(start, TAU), (0.0, end - TAU)]
  }
}

pub fn expanded(b: Bounds, r: f64) -> Bounds {
  Bounds {
    min: Point::new(b.min.x - r, b.min.y - r),
    max: Point::new(b.max.x + r, b.max.y + r),
  }
}
pub fn distance(a: Point, b: Point) -> f64 {
  (a.x - b.x).hypot(a.y - b.y)
}
pub fn sub(a: Point, b: Point) -> Point {
  Point::new(a.x - b.x, a.y - b.y)
}
fn add(a: Point, b: Point) -> Point {
  Point::new(a.x + b.x, a.y + b.y)
}
fn mul(a: Point, s: f64) -> Point {
  Point::new(a.x * s, a.y * s)
}
fn cross(a: Point, b: Point) -> f64 {
  a.x * b.y - a.y * b.x
}
fn dot(a: Point, b: Point) -> f64 {
  a.x * b.x + a.y * b.y
}
