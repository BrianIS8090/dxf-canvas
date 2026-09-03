use crate::geometry::{Bounds, DrawingItem, Point};

const MIN_DIMENSION: f64 = 1.0;

#[derive(Clone, Copy, Debug)]
struct PackedRect {
  index: usize,
  width: f64,
  height: f64,
}

#[derive(Clone, Copy, Debug)]
struct OccupiedRect {
  min_x: f64,
  min_y: f64,
  max_x: f64,
  max_y: f64,
}

impl OccupiedRect {
  fn intersects(self, other: Self, gap: f64) -> bool {
    self.min_x < other.max_x + gap
      && self.max_x + gap > other.min_x
      && self.min_y < other.max_y + gap
      && self.max_y + gap > other.min_y
  }
}

pub fn arrange(items: &mut [DrawingItem], viewport_aspect: f64) -> Option<Bounds> {
  if items.is_empty() {
    return None;
  }

  let typical_size = items
    .iter()
    .map(|item| {
      let bounds = item.scaled_bounds();
      bounds.width().max(bounds.height())
    })
    .filter(|value| value.is_finite() && *value > 0.0)
    .sum::<f64>()
    / items.len().max(1) as f64;
  let gap = (typical_size * 0.06).clamp(8.0, 180.0);
  let label_band = (typical_size * 0.075).clamp(16.0, 220.0);

  let mut rectangles: Vec<_> = items
    .iter()
    .enumerate()
    .map(|(index, item)| {
      let bounds = item.scaled_bounds();
      PackedRect {
        index,
        width: bounds.width().max(MIN_DIMENSION),
        height: bounds.height().max(MIN_DIMENSION) + label_band,
      }
    })
    .collect();
  rectangles.sort_by(|left, right| {
    let left_area = left.width * left.height;
    let right_area = right.width * right.height;
    right_area.total_cmp(&left_area)
  });

  let total_area = rectangles
    .iter()
    .map(|rect| (rect.width + gap) * (rect.height + gap))
    .sum::<f64>();
  let max_width = rectangles.iter().map(|rect| rect.width).fold(0.0, f64::max);
  let aspect = viewport_aspect.clamp(0.75, 2.4);
  let target_width = (total_area * aspect).sqrt().max(max_width);

  let mut occupied: Vec<OccupiedRect> = Vec::with_capacity(rectangles.len());
  for rect in rectangles {
    let mut x_candidates = vec![0.0];
    for placed in &occupied {
      x_candidates.push(placed.max_x + gap);
    }
    x_candidates.sort_by(f64::total_cmp);
    x_candidates.dedup_by(|left, right| (*left - *right).abs() < 0.001);

    let mut best: Option<(f64, f64, f64)> = None;
    for x in x_candidates {
      if x + rect.width > target_width + gap && x > 0.0 {
        continue;
      }

      let mut y = 0.0;
      loop {
        let candidate = OccupiedRect {
          min_x: x,
          min_y: y,
          max_x: x + rect.width,
          max_y: y + rect.height,
        };
        let collision = occupied
          .iter()
          .filter(|placed| candidate.intersects(**placed, gap))
          .map(|placed| placed.max_y + gap)
          .max_by(f64::total_cmp);
        match collision {
          Some(next_y) if next_y > y + 0.001 => y = next_y,
          _ => break,
        }
      }

      let resulting_width = occupied
        .iter()
        .map(|placed| placed.max_x)
        .fold(x + rect.width, f64::max);
      let resulting_height = occupied
        .iter()
        .map(|placed| placed.max_y)
        .fold(y + rect.height, f64::max);
      let aspect_penalty = ((resulting_width / resulting_height.max(1.0)) / aspect)
        .ln()
        .abs();
      let score = resulting_height + aspect_penalty * target_width * 0.25 + x * 0.0001;

      if best.is_none_or(|current| score < current.0) {
        best = Some((score, x, y));
      }
    }

    let (_, x, y) = best.unwrap_or((0.0, 0.0, occupied.last().map_or(0.0, |r| r.max_y + gap)));
    let item = &mut items[rect.index];
    let scaled_bounds = item.scaled_bounds();
    item.offset = Point::new(
      x - scaled_bounds.min.x,
      -(y + label_band) - scaled_bounds.max.y,
    );
    occupied.push(OccupiedRect {
      min_x: x,
      min_y: y,
      max_x: x + rect.width,
      max_y: y + rect.height,
    });
  }

  let mut result = Bounds::empty();
  for item in items.iter() {
    result.include_bounds(item.placed_bounds());
  }
  result.is_valid().then_some(result)
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use crate::geometry::{DrawingItem, Primitive};

  use super::*;

  fn item(name: &str, width: f64, height: f64) -> DrawingItem {
    DrawingItem {
      units: Default::default(),
      path: PathBuf::from(name),
      name: name.to_owned(),
      primitives: vec![Primitive::Path {
        curves: vec![],
        points: vec![Point::new(0.0, 0.0), Point::new(width, height)],
        closed: false,
      }],
      bounds: Bounds {
        min: Point::new(0.0, 0.0),
        max: Point::new(width, height),
      },
      offset: Point::default(),
      scale: 1.0,
      unsupported_entities: 0,
    }
  }

  #[test]
  fn arrangement_does_not_overlap_objects() {
    let mut items = vec![
      item("large.dxf", 1200.0, 900.0),
      item("wide.dxf", 800.0, 120.0),
      item("small.dxf", 180.0, 90.0),
    ];
    arrange(&mut items, 16.0 / 9.0).unwrap();

    for left in 0..items.len() {
      for right in (left + 1)..items.len() {
        let a = items[left].placed_bounds();
        let b = items[right].placed_bounds();
        let overlaps =
          a.min.x < b.max.x && a.max.x > b.min.x && a.min.y < b.max.y && a.max.y > b.min.y;
        assert!(!overlaps, "объекты {left} и {right} пересеклись");
      }
    }
  }
}
