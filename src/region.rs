use std::collections::{HashMap, VecDeque};

use crate::{
  geometry::{Bounds, DrawingItem, Point},
  planar::{ContactKind, Contacts, Edge, EdgeShape, contacts, distance, drawing_edges},
  spatial::SpatialIndex,
};

#[derive(Clone, Debug)]
pub struct RegionMeasurement {
  pub area: f64,
  pub perimeter: f64,
  pub holes: usize,
  pub approximate: bool,
  pub boundaries: Vec<Vec<Point>>,
  pub slit_count: usize,
  pub slit_length: f64,
  pub slits: Vec<Vec<Point>>,
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
  let graph = EndpointGraph::new(&edges.values, tolerance);
  let boundaries = boundaries(&edges.values, &graph)?;
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
  let intersections = contacts(&edges.values, 0.001 / item.units.factor());
  if intersections.limited {
    return Err(
      "Проверка пересечений достигла защитного лимита. Сузьте набор видимых слоёв.".into(),
    );
  }
  if intersections
    .values
    .iter()
    .any(|c| owner[c.a] && owner[c.b])
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
    slit_count: 0,
    slit_length: 0.0,
    slits: Vec::new(),
  };
  let mut comparisons = 0;
  for &i in &selected {
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
  add_slits(
    &edges.values,
    &graph,
    &intersections,
    &selected.iter().map(|i| &boundaries[*i]).collect::<Vec<_>>(),
    &owner,
    tolerance,
    &mut result,
  )?;
  if result.area <= 0.0 || !result.area.is_finite() || !result.perimeter.is_finite() {
    return Err("Некорректная площадь или длина контура.".into());
  }
  Ok(result)
}

struct EndpointGraph {
  positions: Vec<Point>,
  nodes: Vec<[usize; 2]>,
  adjacent: Vec<Vec<usize>>,
}

impl EndpointGraph {
  fn new(edges: &[Edge], tolerance: f64) -> Self {
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
    Self {
      positions,
      nodes,
      adjacent,
    }
  }

  fn closed_core(&self) -> Vec<bool> {
    // Отделяем тупиковые прорези от границ; исходные участки сохраняются для длины реза.
    let mut active = vec![true; self.nodes.len()];
    let mut degree: Vec<_> = self.adjacent.iter().map(Vec::len).collect();
    let mut queue: VecDeque<_> = (0..degree.len()).filter(|i| degree[*i] < 2).collect();
    while let Some(node) = queue.pop_front() {
      for &edge in &self.adjacent[node] {
        if !active[edge] {
          continue;
        }
        active[edge] = false;
        for endpoint in self.nodes[edge] {
          degree[endpoint] -= 1;
          if degree[endpoint] == 1 {
            queue.push_back(endpoint);
          }
        }
      }
    }
    active
  }
}

fn boundaries(edges: &[Edge], graph: &EndpointGraph) -> Result<Vec<Boundary>, String> {
  let nodes = &graph.nodes;
  let active = graph.closed_core();
  let adjacent: Vec<Vec<usize>> = graph
    .adjacent
    .iter()
    .map(|neighbors| neighbors.iter().copied().filter(|i| active[*i]).collect())
    .collect();
  let mut visited = vec![false; edges.len()];
  let mut result = Vec::new();
  let mut total_points = 0;
  for first in 0..edges.len() {
    if visited[first] || !active[first] {
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

fn add_slits(
  edges: &[Edge],
  graph: &EndpointGraph,
  contacts: &Contacts,
  contours: &[&Boundary],
  owner: &[bool],
  tolerance: f64,
  result: &mut RegionMeasurement,
) -> Result<(), String> {
  let error = "Внутри детали есть незамкнутый или неоднозначный контур. Прорезь должна одним концом примыкать к границе, а другим заканчиваться в материале, без пересечений и разветвлений.";
  let outer = contours
    .iter()
    .max_by(|a, b| a.area.total_cmp(&b.area))
    .unwrap();
  let mut edge_contacts = vec![Vec::new(); edges.len()];
  for (index, contact) in contacts.values.iter().enumerate() {
    edge_contacts[contact.a].push(index);
    edge_contacts[contact.b].push(index);
  }
  let mut visited = owner.to_vec();
  let mut visited_nodes = vec![false; graph.positions.len()];
  let open_degree: Vec<_> = graph
    .adjacent
    .iter()
    .map(|neighbors| neighbors.iter().filter(|i| !owner[**i]).count())
    .collect();
  let contour_index = SpatialIndex::new(
    contours.len(),
    contours.iter().enumerate().map(|(i, b)| (i, b.bounds)),
  );
  let mut sample_count = 0;
  let mut comparisons = 0;
  for first in 0..edges.len() {
    if visited[first] {
      continue;
    }
    let mut component = Vec::new();
    let mut stack = vec![first];
    visited[first] = true;
    while let Some(edge) = stack.pop() {
      component.push(edge);
      for node in graph.nodes[edge] {
        if visited_nodes[node] {
          continue;
        }
        visited_nodes[node] = true;
        for &neighbor in &graph.adjacent[node] {
          if !visited[neighbor] {
            visited[neighbor] = true;
            stack.push(neighbor);
          }
        }
      }
    }
    let relevant = component.iter().any(|i| {
      let (a, b) = edges[*i].shape.ends();
      (contains_bounds(outer.bounds, a) && inside(&outer.points, a))
        || (contains_bounds(outer.bounds, b) && inside(&outer.points, b))
        || graph.nodes[*i]
          .iter()
          .any(|n| graph.adjacent[*n].iter().any(|e| owner[*e]))
        || edge_contacts[*i].iter().any(|c| {
          let c = &contacts.values[*c];
          owner[c.a] || owner[c.b]
        })
    });
    if !relevant {
      continue;
    }
    let mut nodes: Vec<_> = component.iter().flat_map(|i| graph.nodes[*i]).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let mut ends = Vec::new();
    for &node in &nodes {
      let degree = open_degree[node];
      if degree == 1 {
        ends.push(node);
      }
      if degree > 2 {
        return Err(error.into());
      }
    }
    if ends.len() != 2 {
      return Err(error.into());
    }
    let mut attachments = Vec::new();
    for &node in &nodes {
      if graph.adjacent[node].iter().any(|i| owner[*i]) {
        if !ends.contains(&node) {
          return Err(error.into());
        }
        attachments.push(node);
      }
    }
    for &edge in &component {
      for &index in &edge_contacts[edge] {
        let contact = &contacts.values[index];
        let other = if contact.a == edge {
          contact.b
        } else {
          contact.a
        };
        let ContactKind::Crossing(point) = contact.kind else {
          return Err(error.into());
        };
        if !owner[other] {
          return Err(error.into());
        }
        let Some(&node) = ends
          .iter()
          .find(|n| distance(graph.positions[**n], point) <= tolerance)
        else {
          return Err(error.into());
        };
        attachments.push(node);
      }
    }
    attachments.sort_unstable();
    attachments.dedup();
    if attachments.len() != 1 {
      return Err(error.into());
    }
    for edge in component {
      let shape = edges[edge].shape;
      let points = shape.points();
      sample_count += points.len();
      if sample_count > 2_000_000 {
        return Err("Слишком много точек прорезей. Оставьте видимой только нужную деталь.".into());
      }
      // Прорезь лежит в материале, а не в отверстии или снаружи. Ширина реза в DXF не задана.
      for pair in points.windows(2) {
        let midpoint = Point::new((pair[0].x + pair[1].x) * 0.5, (pair[0].y + pair[1].y) * 0.5);
        let mut depth = 0;
        for index in contour_index.query(Bounds {
          min: midpoint,
          max: midpoint,
        }) {
          comparisons += contours[index].points.len();
          if comparisons > 20_000_000 {
            return Err("Проверка расположения прорезей достигла защитного лимита. Оставьте только нужную деталь.".into());
          }
          depth += usize::from(inside(&contours[index].points, midpoint));
        }
        if depth % 2 == 0 {
          return Err(error.into());
        }
      }
      result.slit_length += shape.length();
      result.perimeter += shape.length();
      result.approximate |= edges[edge].approximate;
      let (a, b) = shape.ends();
      for (side, p) in [a, b].into_iter().enumerate() {
        result.approximate |= distance(p, graph.positions[graph.nodes[edge][side]]) > 1e-9;
      }
      result.slits.push(points);
    }
    result.slit_count += 1;
  }
  Ok(())
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
