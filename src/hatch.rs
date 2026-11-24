use std::f64::consts::TAU;

use crate::{
  geometry::{Bounds, Point},
  raw_dxf::Record,
};

#[derive(Clone, Debug)]
pub struct Hatch {
  pub loops: Vec<Vec<Point>>,
  pub solid: bool,
  pub style: i32,
  pub patterns: Vec<Pattern>,
}

#[derive(Clone, Debug)]
pub struct Pattern {
  pub angle: f64,
  pub base: Point,
  pub offset: Point,
  pub dashes: Vec<f64>,
}

#[derive(Clone, Debug)]
pub struct PatternHatch {
  pub source: Hatch,
  patterns: Vec<WindowPattern>,
}

#[derive(Clone, Debug)]
struct WindowPattern {
  sin: f64,
  cos: f64,
  step: f64,
  offset_x: f64,
  period: f64,
  rings: Vec<Vec<Point>>,
  bounds: Bounds,
  edge_count: usize,
}

pub struct PatternBudget {
  pub segments: usize,
  pub work: usize,
}

impl PatternBudget {
  fn spend(&mut self, amount: usize) -> bool {
    if amount > self.work {
      self.work = 0;
      return false;
    }
    self.work -= amount;
    true
  }
}

impl PatternHatch {
  // None означает обзорный режим: ни частичный рисунок, ни непроверенный результат не выдаются.
  pub fn lines_in_view(
    &self,
    window: [Point; 4],
    budget: &mut PatternBudget,
  ) -> Option<Vec<[Point; 2]>> {
    if window.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
      return None;
    }
    let mut result = Vec::new();
    for (pattern, prepared) in self.source.patterns.iter().zip(&self.patterns) {
      let project = |p: Point| {
        Point::new(
          (p.x - pattern.base.x) * prepared.cos + (p.y - pattern.base.y) * prepared.sin,
          -(p.x - pattern.base.x) * prepared.sin + (p.y - pattern.base.y) * prepared.cos,
        )
      };
      let clip = window.map(project);
      let bounds = Bounds::from_points(clip)?;
      let min_y = bounds.min.y.max(prepared.bounds.min.y);
      let max_y = bounds.max.y.min(prepared.bounds.max.y);
      if min_y > max_y {
        continue;
      }
      let first = (min_y / prepared.step).min(max_y / prepared.step).ceil();
      let last = (min_y / prepared.step).max(max_y / prepared.step).floor();
      if !first.is_finite()
        || !last.is_finite()
        || first.abs().max(last.abs()) > (1_u64 << 53) as f64
      {
        return None;
      }
      if first > last {
        continue;
      }
      let rows = (last - first + 1.0) as usize;
      if rows > budget.work / prepared.edge_count.saturating_add(4).max(1) {
        return None;
      }
      for row in first as i64..=last as i64 {
        if !budget.spend(prepared.edge_count + 4) {
          return None;
        }
        let y = row as f64 * prepared.step;
        let mut clip_hits = Vec::with_capacity(4);
        intersections(&clip, y, &mut clip_hits);
        if clip_hits.len() < 2 {
          continue;
        }
        clip_hits.sort_by(f64::total_cmp);
        let left = clip_hits[0];
        let right = *clip_hits.last()?;
        let mut hits = Vec::new();
        for ring in &prepared.rings {
          intersections(ring, y, &mut hits);
        }
        hits.sort_by(f64::total_cmp);
        let unproject = |x: f64| {
          Point::new(
            pattern.base.x + x * prepared.cos - y * prepared.sin,
            pattern.base.y + x * prepared.sin + y * prepared.cos,
          )
        };
        let mut emit = |a: f64, b: f64| -> Option<()> {
          if budget.segments == 0 {
            return None;
          }
          let segment = [unproject(a), unproject(b)];
          if segment.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return None;
          }
          budget.segments -= 1;
          result.push(segment);
          Some(())
        };
        for pair in hits.as_chunks::<2>().0 {
          let a = pair[0].max(left);
          let b = pair[1].min(right);
          if a >= b {
            continue;
          }
          if pattern.dashes.is_empty() {
            emit(a, b)?;
            continue;
          }
          // Фаза берётся из исходной строки, а не из левого края текущего окна.
          let phase = (row as f64 * prepared.offset_x).rem_euclid(prepared.period);
          let first_cycle = ((a - phase) / prepared.period).floor();
          let last_cycle = ((b - phase) / prepared.period).ceil() - 1.0;
          if !first_cycle.is_finite()
            || !last_cycle.is_finite()
            || first_cycle.abs().max(last_cycle.abs()) > (1_u64 << 53) as f64
          {
            return None;
          }
          let cycles = (last_cycle - first_cycle + 1.0) as usize;
          let operations = cycles.checked_mul(pattern.dashes.len())?;
          if operations > budget.work {
            return None;
          }
          budget.work -= operations;
          for cycle in first_cycle as i64..=last_cycle as i64 {
            let mut x = phase + cycle as f64 * prepared.period;
            for dash in &pattern.dashes {
              let next = x + dash.abs();
              if *dash >= 0.0 && next >= a && x <= b {
                emit(x.max(a), next.min(b))?;
              }
              if *dash != 0.0 && next <= x {
                return None;
              }
              x = next;
            }
          }
        }
      }
    }
    Some(result)
  }
}

fn intersections(ring: &[Point], y: f64, hits: &mut Vec<f64>) {
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
    } else if !(path.integer(92, 0) & 2 != 0
      && path.integer(73, 0) == 1
      && path.integer(93, 0) == 2
      && path
        .pairs
        .iter()
        .filter(|(code, _)| matches!(*code, 10 | 20))
        .all(|(_, value)| value.trim().parse::<f64>().is_ok_and(f64::is_finite))
      && path
        .pairs
        .iter()
        .filter(|(code, _)| *code == 42)
        .all(|(_, value)| value.trim().parse::<f64>() == Ok(0.0)))
    {
      return Err("Не удалось восстановить все границы штриховки".to_owned());
    }
  }
  // Замкнутый отрезок A→B→A имеет нулевую площадь: он не вырезает отверстие и не отменяет внешний контур.
  if starts.is_empty() || starts.len() != record.integer(91, 0) as usize {
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
  pub fn exceeds_eager_limit(error: &str) -> bool {
    matches!(
      error,
      "Слишком плотная штриховка (более 100 000 линий)" | "Слишком много штрихов в одной штриховке"
    )
  }

  pub fn prepare_view(&self) -> Result<PatternHatch, String> {
    let depths = Self::depths(&self.parents());
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
    let mut patterns = Vec::new();
    for pattern in &self.patterns {
      if ![
        pattern.angle,
        pattern.base.x,
        pattern.base.y,
        pattern.offset.x,
        pattern.offset.y,
      ]
      .iter()
      .all(|v| v.is_finite())
        || pattern.dashes.iter().any(|v| !v.is_finite())
      {
        return Err("Неконечные параметры штриховки".to_owned());
      }
      let (sin, cos) = pattern.angle.sin_cos();
      let step = -pattern.offset.x * sin + pattern.offset.y * cos;
      let period: f64 = pattern.dashes.iter().map(|v| v.abs()).sum();
      if step.abs() < 1e-9 || (!pattern.dashes.is_empty() && period < 1e-9) {
        return Err("Нулевой шаг или период штриховки".to_owned());
      }
      let projected: Vec<Vec<_>> = rings
        .iter()
        .map(|ring| {
          ring
            .iter()
            .map(|p| {
              Point::new(
                (p.x - pattern.base.x) * cos + (p.y - pattern.base.y) * sin,
                -(p.x - pattern.base.x) * sin + (p.y - pattern.base.y) * cos,
              )
            })
            .collect()
        })
        .collect();
      if projected
        .iter()
        .flatten()
        .any(|p| !p.x.is_finite() || !p.y.is_finite())
      {
        return Err("Неконечная граница штриховки".to_owned());
      }
      let bounds =
        Bounds::from_points(projected.iter().flatten().copied()).ok_or("Пустая штриховка")?;
      let edge_count = projected.iter().map(Vec::len).sum();
      patterns.push(WindowPattern {
        sin,
        cos,
        step,
        offset_x: pattern.offset.x * cos + pattern.offset.y * sin,
        period,
        rings: projected,
        bounds,
        edge_count,
      });
    }
    Ok(PatternHatch {
      source: self.clone(),
      patterns,
    })
  }

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

  #[cfg(test)]
  pub fn lines(&self) -> Result<Vec<[Point; 2]>, String> {
    self.lines_with_limit(200_000)
  }

  pub fn lines_with_limit(&self, limit: usize) -> Result<Vec<[Point; 2]>, String> {
    if self.loops.is_empty() {
      return Ok(Vec::new());
    }
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
    let mut remaining_work = 2_000_000_usize;
    for pattern in &self.patterns {
      // Полностью отрицательное определение состоит только из пробелов и ничего не рисует.
      if !pattern.dashes.is_empty() && pattern.dashes.iter().all(|dash| *dash < 0.0) {
        continue;
      }
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
      let row_work = projected
        .iter()
        .map(Vec::len)
        .sum::<usize>()
        .saturating_add(1);
      for row in first..=last {
        if row_work > remaining_work {
          return Err("Слишком много штрихов в одной штриховке".to_owned());
        }
        remaining_work -= row_work;
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
            if result.len() == limit {
              return Err("Слишком много штрихов в одной штриховке".to_owned());
            }
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
                if remaining_work == 0 {
                  return Err("Слишком много штрихов в одной штриховке".to_owned());
                }
                remaining_work -= 1;
                let next = x + dash.abs();
                // Ненулевой шаг обязан продвигать координату и после округления f64.
                if *dash != 0.0 && next <= x {
                  return Err("Слишком много штрихов в одной штриховке".to_owned());
                }
                if *dash >= 0.0 && next >= pair[0] && x <= pair[1] {
                  if result.len() == limit {
                    return Err("Слишком много штрихов в одной штриховке".to_owned());
                  }
                  result.push([unproject(x.max(pair[0])), unproject(next.min(pair[1]))]);
                }
                x = next;
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
  fn eager_negative_pattern_and_stalled_float_steps_finish_within_a_bound() {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
      let source = |start: f64, dashes: Vec<f64>| Hatch {
        loops: vec![vec![
          Point::new(start, 0.0),
          Point::new(start + 100.0, 0.0),
          Point::new(start + 100.0, 1.0),
          Point::new(start, 1.0),
        ]],
        solid: false,
        style: 0,
        patterns: vec![Pattern {
          angle: 0.0,
          base: Point::default(),
          offset: Point::new(0.0, 2.0),
          dashes,
        }],
      };
      let empty = source(1e16, vec![-1.0]).lines();
      let stalled = source(1e16, vec![1.0, -1.0]).lines();
      let _ = send.send((empty, stalled));
    });
    let (empty, stalled) = receive
      .recv_timeout(std::time::Duration::from_secs(1))
      .expect(
        "Построение не завершилось: пустой рисунок или непродвигающийся шаг зациклили импорт",
      );
    assert!(empty.unwrap().is_empty());
    assert!(Hatch::exceeds_eager_limit(&stalled.unwrap_err()));
  }

  #[test]
  fn eager_work_budget_limits_gap_traversal_before_the_output_limit() {
    let mut dashes = vec![-1.0; 2048];
    dashes.push(1.0);
    let source = Hatch {
      loops: vec![vec![
        Point::new(0.0, 0.0),
        Point::new(10_000_000.0, 0.0),
        Point::new(10_000_000.0, 1.0),
        Point::new(0.0, 1.0),
      ]],
      solid: false,
      style: 0,
      patterns: vec![Pattern {
        angle: 0.0,
        base: Point::default(),
        offset: Point::new(0.0, 2.0),
        dashes,
      }],
    };
    // Видимых отрезков меньше 5000; ограничиться только result.len() недостаточно.
    assert!(Hatch::exceeds_eager_limit(&source.lines().unwrap_err()));
    let prepared = source.prepare_view().unwrap();
    let mut budget = PatternBudget {
      segments: 10,
      work: 10_000,
    };
    let detail = prepared
      .lines_in_view(
        Bounds {
          min: Point::new(2047.0, -0.5),
          max: Point::new(2050.0, 0.5),
        }
        .corners(),
        &mut budget,
      )
      .unwrap();
    assert_eq!(detail, [[Point::new(2048.0, 0.0), Point::new(2049.0, 0.0)]]);
  }
  fn window_hatch() -> Hatch {
    Hatch {
      loops: vec![
        vec![
          Point::new(0.0, 0.0),
          Point::new(100.0, 0.0),
          Point::new(100.0, 100.0),
          Point::new(0.0, 100.0),
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
        base: Point::new(1.0, 0.0),
        offset: Point::new(1.0, 2.0),
        dashes: vec![2.0, -3.0],
      }],
    }
  }

  #[test]
  fn view_window_preserves_dash_phase_and_holes_when_panning() {
    let geometry = window_hatch().prepare_view().unwrap();
    let window = Bounds {
      min: Point::new(3.0, 3.0),
      max: Point::new(8.5, 8.5),
    };
    let mut budget = PatternBudget {
      segments: 100,
      work: 1000,
    };
    let actual = geometry
      .lines_in_view(window.corners(), &mut budget)
      .unwrap();
    assert_eq!(
      actual,
      vec![
        [Point::new(3.0, 4.0), Point::new(4.0, 4.0)],
        [Point::new(8.0, 4.0), Point::new(8.5, 4.0)],
        [Point::new(4.0, 6.0), Point::new(6.0, 6.0)],
        [Point::new(5.0, 8.0), Point::new(7.0, 8.0)]
      ]
    );
    let shifted = Bounds {
      min: Point::new(3.5, 3.0),
      ..window
    };
    let mut budget = PatternBudget {
      segments: 100,
      work: 1000,
    };
    let panned = geometry
      .lines_in_view(shifted.corners(), &mut budget)
      .unwrap();
    assert_eq!(panned[0], [Point::new(3.5, 4.0), Point::new(4.0, 4.0)]);
    assert_eq!(panned[1..], actual[1..]);
  }

  #[test]
  fn view_window_budget_prevents_full_materialization_and_nonfinite_windows() {
    let mut source = window_hatch();
    source.patterns[0].offset = Point::new(0.0, 0.0001);
    source.patterns[0].dashes = vec![0.0001, -0.0001];
    let geometry = source.prepare_view().unwrap();
    let mut budget = PatternBudget {
      segments: 100,
      work: 1000,
    };
    assert!(
      geometry
        .lines_in_view(
          Bounds {
            min: Point::new(0.0, 0.0),
            max: Point::new(100.0, 100.0)
          }
          .corners(),
          &mut budget
        )
        .is_none()
    );
    assert!(budget.segments <= 100 && budget.work <= 1000);
    assert!(
      geometry
        .lines_in_view([Point::new(f64::INFINITY, 0.0); 4], &mut budget)
        .is_none()
    );
    let mut budget = PatternBudget {
      segments: 100,
      work: 1000,
    };
    let detail = geometry
      .lines_in_view(
        Bounds {
          min: Point::new(0.0, 0.0),
          max: Point::new(0.0005, 0.0005),
        }
        .corners(),
        &mut budget,
      )
      .unwrap();
    assert!(!detail.is_empty() && detail.len() <= 100);
  }

  #[test]
  fn rotated_window_and_negative_step_keep_exact_clipping() {
    let mut source = window_hatch();
    source.patterns[0].offset.y = -2.0;
    let prepared = source.prepare_view().unwrap();
    let bounds = Bounds {
      min: Point::new(3.0, 3.0),
      max: Point::new(8.5, 8.5),
    };
    let mut budget = PatternBudget {
      segments: 1000,
      work: 10000,
    };
    let actual = prepared
      .lines_in_view(bounds.corners(), &mut budget)
      .unwrap();
    let expected: Vec<_> = source
      .lines()
      .unwrap()
      .into_iter()
      .filter_map(|line| {
        if line[0].y < 3.0 || line[0].y >= 8.5 {
          return None;
        }
        let a = line[0].x.max(3.0);
        let b = line[1].x.min(8.5);
        (a <= b).then_some([Point::new(a, line[0].y), Point::new(b, line[1].y)])
      })
      .collect();
    assert_eq!(actual, expected);
    let diamond = [
      Point::new(0.0, 5.0),
      Point::new(5.0, 0.0),
      Point::new(10.0, 5.0),
      Point::new(5.0, 10.0),
    ];
    let mut budget = PatternBudget {
      segments: 1000,
      work: 10000,
    };
    let clipped = prepared.lines_in_view(diamond, &mut budget).unwrap();
    assert!(!clipped.is_empty());
    for line in clipped {
      assert!(
        line
          .iter()
          .all(|p| (p.x - 5.0).abs() + (p.y - 5.0).abs() <= 5.0 + 1e-9)
      );
      let midpoint = Point::new((line[0].x + line[1].x) * 0.5, line[0].y);
      assert!(!(midpoint.x > 4.0 && midpoint.x < 6.0 && midpoint.y > 4.0 && midpoint.y < 6.0));
    }
  }

  #[test]
  fn two_vertex_zero_area_loop_does_not_discard_the_outer_boundary() {
    let text = concat!(
      "0\nSECTION\n2\nENTITIES\n0\nHATCH\n70\n0\n91\n2\n",
      "92\n2\n72\n0\n73\n1\n93\n4\n10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n10\n10\n0\n20\n10\n97\n0\n",
      "92\n2\n72\n0\n73\n1\n93\n2\n10\n2\n20\n5\n10\n8\n20\n5\n97\n0\n",
      "75\n0\n78\n1\n53\n0\n43\n0\n44\n5\n45\n0\n46\n20\n79\n0\n0\nENDSEC\n0\nEOF\n"
    );
    let raw = crate::raw_dxf::RawDxf::parse(text);
    let hatch = decode(&raw.extras[""][0]).unwrap();
    assert_eq!(hatch.loops.len(), 1);
    assert_eq!(
      hatch.lines().unwrap(),
      [[Point::new(0.0, 5.0), Point::new(10.0, 5.0)]]
    );
  }

  #[test]
  fn standalone_two_vertex_zero_area_hatch_has_no_visible_lines() {
    let text = concat!(
      "0\nSECTION\n2\nENTITIES\n0\nHATCH\n70\n0\n91\n1\n",
      "92\n2\n72\n0\n73\n1\n93\n2\n10\n2\n20\n5\n10\n8\n20\n5\n97\n0\n",
      "75\n0\n78\n1\n53\n0\n43\n0\n44\n5\n45\n0\n46\n20\n79\n0\n0\nENDSEC\n0\nEOF\n"
    );
    let raw = crate::raw_dxf::RawDxf::parse(text);
    assert!(
      decode(&raw.extras[""][0])
        .unwrap()
        .lines()
        .unwrap()
        .is_empty()
    );
    let mut open = raw.extras[""][0].clone();
    open
      .pairs
      .iter_mut()
      .find(|(code, _)| *code == 73)
      .unwrap()
      .1 = "0".to_owned();
    assert!(decode(&open).is_err());
    let mut invalid = raw.extras[""][0].clone();
    invalid
      .pairs
      .iter_mut()
      .find(|(code, _)| *code == 10)
      .unwrap()
      .1 = "NaN".to_owned();
    assert!(decode(&invalid).is_err());
  }

  #[test]
  fn two_vertex_bulged_circle_is_preserved_and_missing_paths_still_fail() {
    let text = concat!(
      "0\nSECTION\n2\nENTITIES\n0\nHATCH\n70\n1\n91\n1\n",
      "92\n2\n72\n1\n73\n1\n93\n2\n10\n-1\n20\n0\n42\n1\n10\n1\n20\n0\n42\n1\n97\n0\n",
      "75\n0\n0\nENDSEC\n0\nEOF\n"
    );
    let raw = crate::raw_dxf::RawDxf::parse(text);
    let record = &raw.extras[""][0];
    let hatch = decode(record).unwrap();
    assert_eq!(hatch.loops.len(), 1);
    assert!((signed_area(&hatch.loops[0]).abs() - std::f64::consts::PI).abs() < 0.02);
    let mut missing = record.clone();
    missing
      .pairs
      .iter_mut()
      .find(|(code, _)| *code == 91)
      .unwrap()
      .1 = "2".to_owned();
    assert!(decode(&missing).is_err());
  }

  #[test]
  #[ignore = "Требует локальной папки DXF_CANVAS_HATCH_AUDIT_DIR с частными DXF"]
  fn audit_private_hatch_failures() {
    let directory = std::env::var("DXF_CANVAS_HATCH_AUDIT_DIR").unwrap();
    for index in 0..5 {
      let path = std::path::Path::new(&directory).join(format!("{index}.dxf"));
      let text = std::fs::read_to_string(path).unwrap();
      let raw = crate::raw_dxf::RawDxf::parse(&text);
      for record in raw.extras.values().flatten().filter(|r| r.kind == "HATCH") {
        match decode(record) {
          Err(error) => println!(
            "HATCH_AUDIT {index} {} decode: {error}",
            record.text(5).unwrap_or("?")
          ),
          Ok(hatch) if !hatch.solid => {
            if let Err(error) = hatch.lines() {
              println!(
                "HATCH_AUDIT {index} {} lines: {error}; loops={} vertices={}",
                record.text(5).unwrap_or("?"),
                hatch.loops.len(),
                hatch.loops.iter().map(Vec::len).sum::<usize>()
              );
              let depths = Hatch::depths(&hatch.parents());
              let area: f64 = hatch
                .loops
                .iter()
                .enumerate()
                .map(|(i, ring)| {
                  let area = signed_area(ring).abs();
                  match hatch.style {
                    1 if depths[i] > 1 => 0.0,
                    2 if depths[i] > 0 => 0.0,
                    _ if !depths[i].is_multiple_of(2) => -area,
                    _ => area,
                  }
                })
                .sum();
              let mut estimate = 0.0;
              for p in &hatch.patterns {
                let (s, c) = p.angle.sin_cos();
                let bounds = Bounds::from_points(hatch.loops.iter().flatten().map(|v| {
                  Point::new(
                    (v.x - p.base.x) * c + (v.y - p.base.y) * s,
                    -(v.x - p.base.x) * s + (v.y - p.base.y) * c,
                  )
                }))
                .unwrap();
                let step = -p.offset.x * s + p.offset.y * c;
                let period: f64 = p.dashes.iter().map(|d| d.abs()).sum();
                if period > 0.0 {
                  estimate += area * p.dashes.iter().filter(|d| **d >= 0.0).count() as f64
                    / (step.abs() * period);
                }
                println!(
                  "  pattern angle={} step={} width={} height={} row_estimate={} dashes={:?}",
                  p.angle,
                  step,
                  bounds.width(),
                  bounds.height(),
                  bounds.height() / step.abs(),
                  p.dashes
                );
              }
              println!(
                "  filled_area={area} estimated_segments={estimate:.0} minimum_segment_bytes={:.0}",
                estimate * std::mem::size_of::<[Point; 2]>() as f64
              );
            }
          }
          _ => {}
        }
      }
    }
  }

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
