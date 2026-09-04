use std::f64::consts::TAU;

use crate::{
  geometry::{Bounds, Point},
  raw_dxf::Record,
};

pub struct Hatch {
  pub loops: Vec<Vec<Point>>,
  pub solid: bool,
  pub style: i32,
  pub patterns: Vec<Pattern>,
}

pub struct Pattern {
  pub angle: f64,
  pub base: Point,
  pub offset: Point,
  pub dashes: Vec<f64>,
}

fn xy(record: &Record, code: i16) -> Point {
  Point::new(record.number(code, 0.0), record.number(code + 10, 0.0))
}

fn slice(kind: &str, pairs: &[(i16, String)]) -> Record {
  Record {
    kind: kind.to_owned(),
    pairs: pairs.to_vec(),
  }
}

pub fn decode(record: &Record) -> Result<Hatch, String> {
  let start = record
    .pairs
    .iter()
    .position(|pair| pair.0 == 91)
    .ok_or("У штриховки нет границ")?;
  let end = record.pairs[start + 1..]
    .iter()
    .position(|pair| pair.0 == 75)
    .map(|p| p + start + 1)
    .unwrap_or(record.pairs.len());
  let paths = &record.pairs[start + 1..end];
  let starts: Vec<_> = paths
    .iter()
    .enumerate()
    .filter(|(_, pair)| pair.0 == 92)
    .map(|(i, _)| i)
    .collect();
  let mut loops = Vec::new();
  for (index, offset) in starts.iter().enumerate() {
    let next = starts.get(index + 1).copied().unwrap_or(paths.len());
    let path = slice("boundary", &paths[*offset..next]);
    let mut points = if path.integer(92, 0) & 2 != 0 {
      polyline(&path)?
    } else {
      edges(&path)?
    };
    points.dedup_by(|a, b| (a.x - b.x).hypot(a.y - b.y) < 1.0e-8);
    if points.len() > 1 && distance(points[0], *points.last().unwrap()) < 1.0e-8 {
      points.pop();
    }
    if points.len() >= 3 {
      loops.push(points);
    }
  }
  if starts.len() != record.integer(91, 0) as usize || loops.len() != starts.len() {
    return Err("Не удалось восстановить все границы штриховки".to_owned());
  }
  let mut patterns = Vec::new();
  if let Some(index) = record.pairs.iter().position(|pair| pair.0 == 78) {
    let pairs = &record.pairs[index + 1..];
    let starts: Vec<_> = pairs
      .iter()
      .enumerate()
      .filter(|(_, pair)| pair.0 == 53)
      .map(|(i, _)| i)
      .collect();
    for (i, start) in starts.iter().enumerate() {
      let next = starts.get(i + 1).copied().unwrap_or(pairs.len());
      let line = slice("pattern", &pairs[*start..next]);
      patterns.push(Pattern {
        angle: line.number(53, 0.0).to_radians(),
        base: Point::new(line.number(43, 0.0), line.number(44, 0.0)),
        offset: Point::new(line.number(45, 0.0), line.number(46, 0.0)),
        dashes: line
          .pairs
          .iter()
          .filter(|pair| pair.0 == 49)
          .filter_map(|pair| pair.1.trim().parse().ok())
          .collect(),
      });
    }
  }
  let solid = record.integer(70, 0) == 1;
  if !solid && patterns.is_empty() {
    return Err("У штриховки нет определения рисунка".to_owned());
  }
  if record.integer(450, 0) != 0 {
    return Err("Градиентная штриховка пока не поддерживается".to_owned());
  }
  Ok(Hatch {
    loops,
    solid,
    style: record.integer(75, 0),
    patterns,
  })
}

fn polyline(record: &Record) -> Result<Vec<Point>, String> {
  let mut vertices: Vec<(Point, f64)> = Vec::new();
  for (code, value) in &record.pairs {
    let number = value.trim().parse::<f64>().unwrap_or(0.0);
    match code {
      10 => vertices.push((Point::new(number, 0.0), 0.0)),
      20 => {
        if let Some(last) = vertices.last_mut() {
          last.0.y = number;
        }
      }
      42 => {
        if let Some(last) = vertices.last_mut() {
          last.1 = number;
        }
      }
      _ => {}
    }
  }
  if vertices.len() != record.integer(93, 0) as usize {
    return Err("Неверное число вершин штриховки".to_owned());
  }
  let mut points = Vec::new();
  for i in 0..vertices.len() {
    let (point, bulge) = vertices[i];
    points.push(point);
    if bulge.abs() > 1.0e-12 {
      points.extend(
        crate::dxf_import::sample_bulge(point, vertices[(i + 1) % vertices.len()].0, bulge)
          .into_iter()
          .skip(1),
      );
    }
  }
  Ok(points)
}

fn edges(record: &Record) -> Result<Vec<Point>, String> {
  let starts: Vec<_> = record
    .pairs
    .iter()
    .enumerate()
    .filter(|(_, pair)| pair.0 == 72)
    .map(|(i, _)| i)
    .collect();
  if starts.len() != record.integer(93, 0) as usize {
    return Err("Неверное число рёбер штриховки".to_owned());
  }
  let mut points = Vec::new();
  for (i, start) in starts.iter().enumerate() {
    let end = starts.get(i + 1).copied().unwrap_or(record.pairs.len());
    let edge = slice("edge", &record.pairs[*start..end]);
    let mut edge_points = match edge.integer(72, 0) {
      1 => vec![xy(&edge, 10), xy(&edge, 11)],
      kind @ (2 | 3) => {
        let center = xy(&edge, 10);
        let start = edge.number(50, 0.0).to_radians();
        let end = edge.number(51, 360.0).to_radians();
        let sign = if edge.integer(73, 1) == 1 { 1.0 } else { -1.0 };
        let mut sweep = ((end - start) * sign).rem_euclid(TAU);
        if sweep < 1.0e-12 {
          sweep = TAU;
        }
        let segments = ((sweep / TAU * 144.0).ceil() as usize).max(2);
        let major = if kind == 2 {
          Point::new(edge.number(40, 0.0), 0.0)
        } else {
          xy(&edge, 11)
        };
        let ratio = if kind == 2 { 1.0 } else { edge.number(40, 1.0) };
        (0..=segments)
          .map(|i| {
            let angle = start + sign * sweep * i as f64 / segments as f64;
            Point::new(
              center.x + major.x * angle.cos() - major.y * ratio * angle.sin(),
              center.y + major.y * angle.cos() + major.x * ratio * angle.sin(),
            )
          })
          .collect()
      }
      4 => {
        let mut spline = dxf::entities::Spline {
          degree_of_curve: edge.integer(94, 3),
          ..Default::default()
        };
        for (code, value) in &edge.pairs {
          let n = value.trim().parse::<f64>().unwrap_or(0.0);
          match code {
            40 => spline.knot_values.push(n),
            42 => spline.weight_values.push(n),
            10 => spline.control_points.push(dxf::Point::new(n, 0.0, 0.0)),
            20 => {
              if let Some(last) = spline.control_points.last_mut() {
                last.y = n;
              }
            }
            _ => {}
          }
        }
        crate::dxf_import::sample_spline(&spline)
      }
      _ => return Err("Неизвестное ребро штриховки".to_owned()),
    };
    if let Some(last) = points.last().copied()
      && let (Some(first), Some(end)) = (edge_points.first(), edge_points.last())
      && distance(last, *end) < distance(last, *first)
    {
      edge_points.reverse();
    }
    points.extend(edge_points);
  }
  Ok(points)
}

fn distance(a: Point, b: Point) -> f64 {
  (a.x - b.x).hypot(a.y - b.y)
}

fn signed_area(points: &[Point]) -> f64 {
  points
    .iter()
    .zip(points.iter().cycle().skip(1))
    .take(points.len())
    .map(|(a, b)| a.x * b.y - b.x * a.y)
    .sum::<f64>()
    * 0.5
}

fn contains(ring: &[Point], point: Point) -> bool {
  let mut inside = false;
  for (a, b) in ring
    .iter()
    .zip(ring.iter().cycle().skip(1))
    .take(ring.len())
  {
    if (a.y > point.y) != (b.y > point.y)
      && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
    {
      inside = !inside;
    }
  }
  inside
}

impl Hatch {
  fn parents(&self) -> Vec<Option<usize>> {
    let areas: Vec<_> = self
      .loops
      .iter()
      .map(|ring| signed_area(ring).abs())
      .collect();
    self
      .loops
      .iter()
      .enumerate()
      .map(|(i, ring)| {
        (0..self.loops.len())
          .filter(|j| areas[*j] > areas[i] + 1.0e-8 && contains(&self.loops[*j], ring[0]))
          .min_by(|a, b| areas[*a].total_cmp(&areas[*b]))
      })
      .collect()
  }

  fn depths(parents: &[Option<usize>]) -> Vec<usize> {
    (0..parents.len())
      .map(|i| {
        let mut depth = 0;
        let mut parent = parents[i];
        while let Some(i) = parent {
          depth += 1;
          parent = parents[i];
        }
        depth
      })
      .collect()
  }

  pub fn triangulate(&self) -> Result<(Vec<Point>, Vec<u32>), String> {
    let parents = self.parents();
    let depths = Self::depths(&parents);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (i, ring) in self.loops.iter().enumerate() {
      if !depths[i].is_multiple_of(2) || (self.style != 0 && depths[i] != 0) {
        continue;
      }
      let mut flat: Vec<_> = ring.iter().flat_map(|point| [point.x, point.y]).collect();
      let mut holes = Vec::new();
      if self.style != 2 {
        for (j, parent) in parents.iter().enumerate() {
          if *parent == Some(i) {
            holes.push(flat.len() / 2);
            flat.extend(self.loops[j].iter().flat_map(|point| [point.x, point.y]));
          }
        }
      }
      let triangles =
        earcutr::earcut(&flat, &holes, 2).map_err(|_| "Не удалось построить заливку")?;
      let base = vertices.len() as u32;
      vertices.extend(
        flat
          .as_chunks::<2>()
          .0
          .iter()
          .map(|point| Point::new(point[0], point[1])),
      );
      indices.extend(triangles.into_iter().map(|index| base + index as u32));
    }
    Ok((vertices, indices))
  }

  pub fn lines(&self) -> Result<Vec<[Point; 2]>, String> {
    let parents = self.parents();
    let depths = Self::depths(&parents);
    let rings: Vec<_> = self
      .loops
      .iter()
      .enumerate()
      .filter(|(i, _)| match self.style {
        1 => depths[*i] <= 1,
        2 => depths[*i] == 0,
        _ => true,
      })
      .map(|(_, ring)| ring)
      .collect();
    let mut result = Vec::new();
    for pattern in &self.patterns {
      let (sin, cos) = pattern.angle.sin_cos();
      let project = |point: Point| {
        Point::new(
          (point.x - pattern.base.x) * cos + (point.y - pattern.base.y) * sin,
          -(point.x - pattern.base.x) * sin + (point.y - pattern.base.y) * cos,
        )
      };
      let step = -pattern.offset.x * sin + pattern.offset.y * cos;
      if step.abs() < 1.0e-9 {
        return Err("Нулевой шаг штриховки".to_owned());
      }
      let offset_x = pattern.offset.x * cos + pattern.offset.y * sin;
      let projected: Vec<Vec<_>> = rings
        .iter()
        .map(|ring| ring.iter().copied().map(project).collect())
        .collect();
      let bounds =
        Bounds::from_points(projected.iter().flatten().copied()).ok_or("Пустая штриховка")?;
      let first = (bounds.min.y / step).min(bounds.max.y / step).ceil() as i64;
      let last = (bounds.min.y / step).max(bounds.max.y / step).floor() as i64;
      if last.saturating_sub(first) > 100_000 {
        return Err("Слишком плотная штриховка (более 100 000 линий)".to_owned());
      }
      for row in first..=last {
        let y = row as f64 * step;
        let mut hits = Vec::new();
        for ring in &projected {
          for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
          {
            if (a.y > y) != (b.y > y) {
              hits.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
            }
          }
        }
        hits.sort_by(f64::total_cmp);
        for pair in hits.as_chunks::<2>().0 {
          let unproject = |x: f64| {
            Point::new(
              pattern.base.x + x * cos - y * sin,
              pattern.base.y + x * sin + y * cos,
            )
          };
          if pattern.dashes.is_empty() {
            result.push([unproject(pair[0]), unproject(pair[1])]);
          } else {
            let period: f64 = pattern.dashes.iter().map(|value| value.abs()).sum();
            if period < 1.0e-9 {
              continue;
            }
            let origin = row as f64 * offset_x;
            let mut x = origin + ((pair[0] - origin) / period).floor() * period;
            while x < pair[1] {
              for dash in &pattern.dashes {
                let next = x + dash.abs();
                if *dash >= 0.0 && next >= pair[0] && x <= pair[1] {
                  result.push([unproject(x.max(pair[0])), unproject(next.min(pair[1]))]);
                }
                x = next;
              }
              if result.len() > 200_000 {
                return Err("Слишком много штрихов в одной штриховке".to_owned());
              }
            }
          }
        }
      }
    }
    Ok(result)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn solid_fill_keeps_a_hole_and_a_separate_island() {
    let square = |x, y, size| {
      vec![
        Point::new(x, y),
        Point::new(x + size, y),
        Point::new(x + size, y + size),
        Point::new(x, y + size),
      ]
    };
    let hatch = Hatch {
      loops: vec![
        square(0.0, 0.0, 10.0),
        square(2.0, 2.0, 6.0),
        square(4.0, 4.0, 2.0),
      ],
      solid: true,
      style: 0,
      patterns: vec![],
    };
    let (points, indices) = hatch.triangulate().unwrap();
    let area: f64 = indices
      .as_chunks::<3>()
      .0
      .iter()
      .map(|ids| signed_area(&ids.iter().map(|i| points[*i as usize]).collect::<Vec<_>>()).abs())
      .sum();
    assert!((area - 68.0).abs() < 1.0e-8);
  }
  #[test]
  fn pattern_is_clipped_at_holes() {
    let hatch = Hatch {
      loops: vec![
        vec![
          Point::new(0.0, 0.0),
          Point::new(10.0, 0.0),
          Point::new(10.0, 10.0),
          Point::new(0.0, 10.0),
        ],
        vec![
          Point::new(4.0, 4.0),
          Point::new(6.0, 4.0),
          Point::new(6.0, 6.0),
          Point::new(4.0, 6.0),
        ],
      ],
      solid: false,
      style: 0,
      patterns: vec![Pattern {
        angle: 0.0,
        base: Point::new(0.0, 5.0),
        offset: Point::new(0.0, 20.0),
        dashes: vec![],
      }],
    };
    assert_eq!(
      hatch.lines().unwrap(),
      vec![
        [Point::new(0.0, 5.0), Point::new(4.0, 5.0)],
        [Point::new(6.0, 5.0), Point::new(10.0, 5.0)]
      ]
    );
  }
}
