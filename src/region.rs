use std::collections::HashMap;

use crate::{
  geometry::{Bounds, DrawingItem, Point},
  planar::{Edge, EdgeShape, contacts, distance, drawing_edges},
  spatial::SpatialIndex,
};

#[derive(Clone, Debug)]
pub struct RegionMeasurement {
  pub area: f64,
  pub perimeter: f64,
  pub holes: usize,
  pub approximate: bool,
  pub boundaries: Vec<Vec<Point>>,
}

struct Boundary {
  edges: Vec<usize>,
  points: Vec<Point>,
  bounds: Bounds,
  area: f64,
  perimeter: f64,
  approximate: bool,
}

pub fn measure_region(item: &DrawingItem, point: Point) -> Result<RegionMeasurement, String> {
  if item.unsupported_entities > 0 {
    return Err(
      "Часть сущностей не показана. Достоверную площадь этого файла определить нельзя.".into(),
    );
  }
  let tolerance = 0.01 / item.units.factor();
  let edges = drawing_edges(item);
  if edges.limited {
    return Err(
      "Слишком много участков для расчёта площади. Оставьте видимыми только нужные слои.".into(),
    );
  }
  let boundaries = boundaries(&edges.values, tolerance)?;
  let root = boundaries.iter().enumerate()
    .filter(|(_, b)| contains_bounds(b.bounds, point) && inside(&b.points, point))
    .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
    .map(|(i, _)| i)
    .ok_or("Не найден замкнутый контур под курсором. Щёлкните внутри детали; проверьте разрывы и разветвления.")?;
  let outer = &boundaries[root];
  let selected: Vec<_> = boundaries
    .iter()
    .enumerate()
    .filter(|(i, b)| {
      *i == root || (bounds_inside(outer.bounds, b.bounds) && inside(&outer.points, b.points[0]))
    })
    .map(|(i, _)| i)
    .collect();
  let mut owner = vec![false; edges.values.len()];
  for index in &selected {
    for edge in &boundaries[*index].edges {
      owner[*edge] = true;
    }
  }
  // Не выдаём площадь, если незамкнутое отверстие или ветка остались внутри детали.
  for (i, edge) in edges.values.iter().enumerate() {
    if !owner[i] {
      let (a, b) = edge.shape.ends();
      if inside(&outer.points, a) || inside(&outer.points, b) {
        return Err(
          "Внутри детали есть незамкнутый или разветвлённый контур. Сначала проверьте геометрию."
            .into(),
        );
      }
    }
  }
  let intersections = contacts(&edges.values, 0.001 / item.units.factor());
  if intersections.limited {
    return Err(
      "Проверка пересечений достигла защитного лимита. Сузьте набор видимых слоёв.".into(),
    );
  }
  if intersections
    .values
    .iter()
    .any(|c| owner[c.a] || owner[c.b])
  {
    return Err("Контуры детали пересекаются, касаются или накладываются. Однозначную площадь определить нельзя.".into());
  }
  let index = SpatialIndex::new(
    boundaries.len(),
    selected.iter().map(|i| (*i, boundaries[*i].bounds)),
  );
  let mut result = RegionMeasurement {
    area: 0.0,
    perimeter: 0.0,
    holes: 0,
    approximate: false,
    boundaries: Vec::new(),
  };
  let mut comparisons = 0;
  for i in selected {
    let b = &boundaries[i];
    let p = b.points[0];
    let mut depth = 0;
    for parent in index.query(Bounds { min: p, max: p }) {
      comparisons += 1;
      if comparisons > 2_000_000 {
        return Err("Слишком сложная вложенность контуров. Оставьте только нужную деталь.".into());
      }
      if boundaries[parent].area > b.area && inside(&boundaries[parent].points, p) {
        depth += 1;
      }
    }
    result.area += if depth % 2 == 0 { b.area } else { -b.area };
    result.holes += usize::from(depth % 2 == 1);
    result.perimeter += b.perimeter;
    result.approximate |= b.approximate;
    result.boundaries.push(b.points.clone());
  }
  if result.area <= 0.0 || !result.area.is_finite() || !result.perimeter.is_finite() {
    return Err("Некорректная площадь или длина контура.".into());
  }
  Ok(result)
}

fn boundaries(edges: &[Edge], tolerance: f64) -> Result<Vec<Boundary>, String> {
  let mut positions: Vec<Point> = Vec::new();
  let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
  let mut nodes = Vec::new();
  for edge in edges {
    let (a, b) = edge.shape.ends();
    let mut pair = [0; 2];
    for (side, p) in [a, b].into_iter().enumerate() {
      let key = (
        (p.x / tolerance).floor().clamp(-9e18, 9e18) as i64,
        (p.y / tolerance).floor().clamp(-9e18, 9e18) as i64,
      );
      let found = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |y| (key.0 + x, key.1 + y)))
        .filter_map(|k| buckets.get(&k))
        .flatten()
        .find(|i| distance(positions[**i], p) <= tolerance)
        .copied();
      pair[side] = found.unwrap_or_else(|| {
        let i = positions.len();
        positions.push(p);
        buckets.entry(key).or_default().push(i);
        i
      });
    }
    nodes.push(pair);
  }
  let mut adjacent = vec![Vec::new(); positions.len()];
  for (edge, pair) in nodes.iter().enumerate() {
    for node in pair {
      adjacent[*node].push(edge);
    }
  }
  let mut visited = vec![false; edges.len()];
  let mut result = Vec::new();
  let mut total_points = 0;
  for first in 0..edges.len() {
    if visited[first] {
      continue;
    }
    let start = nodes[first][0];
    let mut node = start;
    let mut current = first;
    let mut indices = Vec::new();
    let mut ordered = Vec::new();
    loop {
      if visited[current] || adjacent[node].len() != 2 {
        break;
      }
      visited[current] = true;
      indices.push(current);
      let forward = nodes[current][0] == node;
      ordered.push(if forward {
        edges[current].shape
      } else {
        edges[current].shape.reversed()
      });
      node = nodes[current][usize::from(forward)];
      if node == start {
        if adjacent[node].len() != 2 {
          break;
        }
        let origin = ordered[0].ends().0;
        let mut area = 0.0;
        let mut perimeter = 0.0;
        let mut points = Vec::new();
        let mut approximate = indices.iter().any(|i| edges[*i].approximate);
        for (i, shape) in ordered.iter().enumerate() {
          area += shape.area_integral(origin);
          perimeter += shape.length();
          let sampled = shape.points();
          total_points += sampled.len();
          if total_points > 2_000_000 {
            return Err("Слишком много криволинейных границ для расчёта площади. Оставьте видимыми только нужную деталь.".into());
          }
          points.extend(sampled.into_iter().skip(usize::from(i > 0)));
          let end = shape.ends().1;
          let next = ordered[(i + 1) % ordered.len()].ends().0;
          let gap = distance(end, next);
          if gap > 1e-9 {
            let bridge = EdgeShape::Line(end, next);
            area += bridge.area_integral(origin);
            perimeter += gap;
            approximate = true;
          }
        }
        if let Some(bounds) = Bounds::from_points(points.iter().copied())
          && area.abs() > 1e-18
        {
          result.push(Boundary {
            edges: indices,
            points,
            bounds,
            area: area.abs(),
            perimeter,
            approximate,
          });
        }
        break;
      }
      if adjacent[node].len() != 2 {
        break;
      }
      current = *adjacent[node].iter().find(|e| **e != current).unwrap();
    }
  }
  Ok(result)
}

fn contains_bounds(b: Bounds, p: Point) -> bool {
  p.x >= b.min.x && p.x <= b.max.x && p.y >= b.min.y && p.y <= b.max.y
}
fn bounds_inside(a: Bounds, b: Bounds) -> bool {
  contains_bounds(a, b.min) && contains_bounds(a, b.max)
}
fn inside(points: &[Point], p: Point) -> bool {
  let mut inside = false;
  if points.is_empty() {
    return false;
  }
  let mut a = points[points.len() - 1];
  for &b in points {
    if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
      inside = !inside;
    }
    a = b;
  }
  inside
}
