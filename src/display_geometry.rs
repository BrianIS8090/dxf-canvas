use crate::geometry::{Bounds, Point, Primitive};

#[derive(Clone, Debug)]
struct Level {
  error: f64,
  points: Vec<Point>,
}

/// Упрощения служат только экрану: измерения и диагностика читают исходные Primitive.
#[derive(Clone, Debug, Default)]
pub struct DisplayGeometry {
  paths: Vec<Vec<Level>>,
}

impl DisplayGeometry {
  pub fn new(primitives: &[Primitive]) -> Self {
    Self {
      paths: primitives
        .iter()
        .map(|primitive| {
          let Primitive::Path { points, closed, .. } = primitive else {
            return vec![];
          };
          if points.len() <= 8 {
            return vec![];
          }
          let Some(bounds) = Bounds::from_points(points.iter().copied()) else {
            return vec![];
          };
          let size = bounds.width().max(bounds.height());
          let importance = importance(points);
          let mut levels = Vec::new();
          let mut previous_len = points.len();
          for divisor in [4096.0, 1024.0, 256.0, 64.0, 16.0, 4.0] {
            let error = size / divisor;
            if error <= 0.0 {
              continue;
            }
            let simplified: Vec<_> = points
              .iter()
              .zip(&importance)
              .filter(|(_, value)| **value >= error)
              .map(|(point, _)| *point)
              .collect();
            let minimum = if *closed { 4 } else { 2 };
            if simplified.len() >= minimum && simplified.len() * 5 < previous_len * 4 {
              previous_len = simplified.len();
              levels.push(Level {
                error,
                points: simplified,
              });
            }
          }
          levels
        })
        .collect(),
    }
  }

  pub fn path<'a>(&'a self, index: usize, source: &'a [Point], tolerance: f64) -> &'a [Point] {
    self
      .paths
      .get(index)
      .and_then(|levels| levels.iter().rev().find(|level| level.error <= tolerance))
      .map_or(source, |level| &level.points)
  }
}

fn segment_distance(point: Point, a: Point, b: Point) -> f64 {
  let dx = b.x - a.x;
  let dy = b.y - a.y;
  let length = dx * dx + dy * dy;
  let t = if length > 0.0 {
    ((point.x - a.x) * dx + (point.y - a.y) * dy) / length
  } else {
    0.0
  }
  .clamp(0.0, 1.0);
  (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy)
}

fn importance(points: &[Point]) -> Vec<f64> {
  let mut result = vec![0.0; points.len()];
  if points.len() < 2 {
    return result;
  }
  result[0] = f64::INFINITY;
  result[points.len() - 1] = f64::INFINITY;
  let mut stack = vec![(0, points.len() - 1, f64::INFINITY)];
  while let Some((start, end, parent_error)) = stack.pop() {
    if end <= start + 1 {
      continue;
    }
    let (index, error) = (start + 1..end)
      .map(|i| (i, segment_distance(points[i], points[start], points[end])))
      .max_by(|a, b| a.1.total_cmp(&b.1))
      .unwrap();
    let error = error.min(parent_error);
    result[index] = error;
    if error > 0.0 {
      stack.push((start, index, error));
      stack.push((index, end, error));
    }
  }
  result
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn display_simplification_respects_tolerance_and_keeps_source_untouched() {
    let points: Vec<_> = (0..=720)
      .map(|i| {
        let angle = i as f64 * std::f64::consts::TAU / 720.0;
        Point::new(100.0 * angle.cos(), 60.0 * angle.sin())
      })
      .collect();
    let primitives = vec![Primitive::Path {
      points: points.clone(),
      closed: true,
      curves: vec![],
    }];
    let cache = DisplayGeometry::new(&primitives);
    for tolerance in [0.01, 0.1, 1.0, 4.0, 15.0] {
      let simplified = cache.path(0, &points, tolerance);
      assert!(simplified.len() >= 4);
      assert_eq!(simplified.first(), points.first());
      assert_eq!(simplified.last(), points.last());
      for point in &points {
        let distance = simplified
          .windows(2)
          .map(|pair| segment_distance(*point, pair[0], pair[1]))
          .min_by(f64::total_cmp)
          .unwrap();
        assert!(
          distance <= tolerance + 1.0e-8,
          "Отклонение {distance} больше {tolerance}"
        );
      }
    }
    assert!(cache.path(0, &points, 4.0).len() < 100);
    let Primitive::Path { points: source, .. } = &primitives[0] else {
      panic!()
    };
    assert_eq!(source, &points);
    assert_eq!(cache.path(0, &points, 0.00001), points.as_slice());
  }

  #[test]
  fn narrow_closed_shapes_are_not_dropped() {
    let points: Vec<_> = (0..=100)
      .map(|i| {
        let angle = i as f64 * std::f64::consts::TAU / 100.0;
        Point::new(100.0 * angle.cos(), 0.001 * angle.sin())
      })
      .collect();
    let cache = DisplayGeometry::new(&[Primitive::Path {
      points: points.clone(),
      closed: true,
      curves: vec![],
    }]);
    assert!(cache.path(0, &points, 100.0).len() >= 4);
  }
}
