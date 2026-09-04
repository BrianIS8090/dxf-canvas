use crate::geometry::{Bounds, Point};

#[derive(Clone, Debug)]
struct Entry {
  bounds: Bounds,
  index: usize,
}

#[derive(Clone, Debug)]
struct Node {
  bounds: Bounds,
  range: std::ops::Range<usize>,
  children: Option<[usize; 2]>,
}

/// Дерево габаритов в исходных координатах; перемещение детали не требует перестройки.
#[derive(Clone, Debug, Default)]
pub struct SpatialIndex {
  nodes: Vec<Node>,
  entries: Vec<Entry>,
  source_len: usize,
}

impl SpatialIndex {
  pub fn new(source_len: usize, bounds: impl IntoIterator<Item = (usize, Bounds)>) -> Self {
    let mut index = Self {
      entries: bounds
        .into_iter()
        .filter(|(_, b)| b.is_valid())
        .map(|(index, bounds)| Entry { index, bounds })
        .collect(),
      source_len,
      nodes: Vec::new(),
    };
    if !index.entries.is_empty() {
      Self::split(&mut index.nodes, &mut index.entries, 0);
    }
    index
  }

  pub fn matches(&self, source_len: usize) -> bool {
    self.source_len == source_len && !self.nodes.is_empty()
  }

  fn split(nodes: &mut Vec<Node>, entries: &mut [Entry], offset: usize) -> usize {
    let mut bounds = Bounds::empty();
    for entry in entries.iter() {
      bounds.include_bounds(entry.bounds);
    }
    let id = nodes.len();
    nodes.push(Node {
      bounds,
      range: offset..offset + entries.len(),
      children: None,
    });
    if entries.len() > 8 {
      let axis_x = bounds.width() >= bounds.height();
      let middle = entries.len() / 2;
      entries.select_nth_unstable_by(middle, |a, b| {
        let coordinate = |b: Bounds| {
          if axis_x {
            b.min.x + b.max.x
          } else {
            b.min.y + b.max.y
          }
        };
        coordinate(a.bounds).total_cmp(&coordinate(b.bounds))
      });
      let (left, right) = entries.split_at_mut(middle);
      let children = [
        Self::split(nodes, left, offset),
        Self::split(nodes, right, offset + middle),
      ];
      nodes[id].children = Some(children);
    }
    id
  }

  pub fn query(&self, bounds: Bounds) -> Vec<usize> {
    let mut result = Vec::new();
    if !self.nodes.is_empty() && bounds.is_valid() {
      self.visit(0, bounds, &mut result);
      // Исходный порядок сохраняет приоритет наложенных контуров и равных привязок.
      result.sort_unstable();
    }
    result
  }

  fn visit(&self, id: usize, bounds: Bounds, result: &mut Vec<usize>) {
    let node = &self.nodes[id];
    if !intersects(node.bounds, bounds) {
      return;
    }
    if contains(bounds, node.bounds) {
      result.extend(
        self.entries[node.range.clone()]
          .iter()
          .map(|entry| entry.index),
      );
    } else if let Some(children) = node.children {
      self.visit(children[0], bounds, result);
      self.visit(children[1], bounds, result);
    } else {
      result.extend(
        self.entries[node.range.clone()]
          .iter()
          .filter(|entry| intersects(entry.bounds, bounds))
          .map(|entry| entry.index),
      );
    }
  }
}

fn intersects(a: Bounds, b: Bounds) -> bool {
  a.min.x <= b.max.x && a.max.x >= b.min.x && a.min.y <= b.max.y && a.max.y >= b.min.y
}

fn contains(a: Bounds, b: Bounds) -> bool {
  a.min.x <= b.min.x && a.max.x >= b.max.x && a.min.y <= b.min.y && a.max.y >= b.max.y
}

pub fn neighborhood(point: Point, radius: f64) -> Bounds {
  Bounds {
    min: Point::new(point.x - radius, point.y - radius),
    max: Point::new(point.x + radius, point.y + radius),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn indexed_regions_match_exhaustive_search_and_preserve_order() {
    let entries: Vec<_> = (0..10_000)
      .map(|i| {
        let point = Point::new(
          (i % 100) as f64 * 10.0 - 500.0,
          (i / 100) as f64 * 10.0 - 500.0,
        );
        (i, neighborhood(point, (i % 7) as f64))
      })
      .collect();
    let index = SpatialIndex::new(entries.len(), entries.iter().copied());
    for i in 0..100 {
      let query = neighborhood(
        Point::new(i as f64 * 12.0 - 600.0, i as f64 * 9.0 - 500.0),
        8.0,
      );
      let expected: Vec<_> = entries
        .iter()
        .filter(|(_, b)| intersects(*b, query))
        .map(|(i, _)| *i)
        .collect();
      assert_eq!(index.query(query), expected);
    }
  }

  #[test]
  fn point_bounds_edges_and_empty_index_are_safe() {
    let point = Point::new(-10.0, 20.0);
    let index = SpatialIndex::new(1, [(0, neighborhood(point, 0.0))]);
    assert_eq!(index.query(neighborhood(point, 0.0)), vec![0]);
    assert!(
      SpatialIndex::default()
        .query(neighborhood(point, 10.0))
        .is_empty()
    );
  }
}
