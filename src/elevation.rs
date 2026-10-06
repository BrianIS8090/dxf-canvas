use std::{
  f64::consts::{PI, TAU},
  ops::Range,
};

use dxf::{
  Point as Point3, Vector,
  entities::{Entity, EntityType, Insert},
};

use crate::{
  cad_scene::EntityStyle,
  diagnostics::{DiagnosticReport, IssueKind, Marker},
  geometry::{Bounds, DrawingItem, Point, RoundCurve},
};

pub const TOLERANCE_MM: f64 = 0.01;

#[derive(Clone, Copy, Debug)]
pub struct ZRange {
  pub min: f64,
  pub max: f64,
  pub approximate: bool,
}

impl ZRange {
  pub fn values(values: impl IntoIterator<Item = f64>) -> Option<Self> {
    let mut result = Self {
      min: f64::INFINITY,
      max: f64::NEG_INFINITY,
      approximate: false,
    };
    for z in values {
      if !z.is_finite() {
        return None;
      }
      result.min = result.min.min(z);
      result.max = result.max.max(z);
    }
    (result.min <= result.max).then_some(result)
  }

  fn center(self) -> f64 {
    self.min + (self.max - self.min) * 0.5
  }
}

#[derive(Clone, Debug)]
pub struct ElevationObject {
  pub source: String,
  pub range: Option<ZRange>,
  pub style: EntityStyle,
  pub bounds: Bounds,
  pub primitives: Range<usize>,
}

// Отдельное преобразование сохраняет исходную высоту, не меняя плоскую отрисовку.
#[derive(Clone, Copy, Debug)]
pub struct Transform3([[f64; 4]; 3]);

impl Transform3 {
  pub const IDENTITY: Self = Self([
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
  ]);

  pub fn then(self, child: Self) -> Self {
    let mut result = [[0.0; 4]; 3];
    for (row, values) in result.iter_mut().enumerate() {
      for (col, value) in values.iter_mut().enumerate() {
        *value = (0..3)
          .map(|k| self.0[row][k] * child.0[k][col])
          .sum::<f64>();
        if col == 3 {
          *value += self.0[row][3];
        }
      }
    }
    Self(result)
  }

  pub fn z(self, p: &Point3) -> f64 {
    self.0[2][0] * p.x + self.0[2][1] * p.y + self.0[2][2] * p.z + self.0[2][3]
  }

  fn vector_z(self, p: &Vector) -> f64 {
    self.0[2][0] * p.x + self.0[2][1] * p.y + self.0[2][2] * p.z
  }

  pub fn ocs(normal: &Vector) -> Self {
    let length = normal.x.hypot(normal.y).hypot(normal.z);
    if !length.is_finite() || length < 1e-12 {
      return Self([[f64::NAN; 4]; 3]);
    }
    let (nx, ny, nz) = (normal.x / length, normal.y / length, normal.z / length);
    let (ax, ay, az) = if nx.abs() < 1.0 / 64.0 && ny.abs() < 1.0 / 64.0 {
      (nz, 0.0, -nx)
    } else {
      (-ny, nx, 0.0)
    };
    let length = ax.hypot(ay).hypot(az);
    let (ax, ay, az) = (ax / length, ay / length, az / length);
    Self([
      [ax, ny * az - nz * ay, nx, 0.0],
      [ay, nz * ax - nx * az, ny, 0.0],
      [az, nx * ay - ny * ax, nz, 0.0],
    ])
  }

  pub fn insert(insert: &Insert, base: &Point3, row: usize, column: usize) -> Self {
    let (sin, cos) = insert.rotation.to_radians().sin_cos();
    let (a, b) = (cos * insert.x_scale_factor, sin * insert.x_scale_factor);
    let (c, d) = (-sin * insert.y_scale_factor, cos * insert.y_scale_factor);
    let dx = column as f64 * insert.column_spacing;
    let dy = row as f64 * insert.row_spacing;
    let local = Self([
      [
        a,
        c,
        0.0,
        insert.location.x + cos * dx - sin * dy - a * base.x - c * base.y,
      ],
      [
        b,
        d,
        0.0,
        insert.location.y + sin * dx + cos * dy - b * base.x - d * base.y,
      ],
      [
        0.0,
        0.0,
        insert.z_scale_factor,
        insert.location.z - insert.z_scale_factor * base.z,
      ],
    ]);
    Self::ocs(&insert.extrusion_direction).then(local)
  }
}

fn sweep(start: f64, end: f64) -> f64 {
  let result = (end - start).rem_euclid(TAU);
  if result.abs() < 1e-12 { TAU } else { result }
}

fn harmonic(center: f64, a: f64, b: f64, start: f64, sweep: f64) -> Option<ZRange> {
  if ![center, a, b, start, sweep].iter().all(|v| v.is_finite()) {
    return None;
  }
  let value = |t: f64| center + a * t.cos() + b * t.sin();
  let mut values = vec![value(start), value(start + sweep)];
  for angle in [b.atan2(a), b.atan2(a) + PI] {
    let distance = if sweep >= 0.0 {
      (angle - start).rem_euclid(TAU)
    } else {
      (start - angle).rem_euclid(TAU)
    };
    if distance <= sweep.abs() + 1e-12 {
      values.push(value(angle));
    }
  }
  ZRange::values(values)
}

fn round_z(transform: Transform3, curve: RoundCurve, elevation: f64) -> Option<ZRange> {
  harmonic(
    transform.z(&Point3::new(curve.center.x, curve.center.y, elevation)),
    transform.0[2][0] * curve.radius,
    transform.0[2][1] * curve.radius,
    curve.start,
    curve.sweep,
  )
}

fn polyline_z(
  vertices: &[(Point, f64)],
  closed: bool,
  elevation: f64,
  transform: Transform3,
) -> Option<ZRange> {
  let mut values: Vec<_> = vertices
    .iter()
    .map(|(p, _)| transform.z(&Point3::new(p.x, p.y, elevation)))
    .collect();
  let count = if closed {
    vertices.len()
  } else {
    vertices.len().saturating_sub(1)
  };
  for i in 0..count {
    let (start, bulge) = vertices[i];
    if !bulge.is_finite() {
      return None;
    }
    if let Some(curve) =
      crate::dxf_import::bulge_round(start, vertices[(i + 1) % vertices.len()].0, bulge)
    {
      let range = round_z(transform, curve, elevation)?;
      values.extend([range.min, range.max]);
    }
  }
  ZRange::values(values)
}

pub fn entity_range(entity: &Entity, transform: Transform3) -> Option<ZRange> {
  let points = |points: &[&Point3], transform: Transform3| {
    ZRange::values(points.iter().map(|p| transform.z(p)))
  };
  match &entity.specific {
    EntityType::Line(line) => points(&[&line.p1, &line.p2], transform),
    EntityType::ModelPoint(point) => points(&[&point.location], transform),
    EntityType::Circle(circle) => round_z(
      transform.then(Transform3::ocs(&circle.normal)),
      RoundCurve {
        center: Point::new(circle.center.x, circle.center.y),
        radius: circle.radius,
        start: 0.0,
        sweep: TAU,
        approximate: false,
      },
      circle.center.z,
    ),
    EntityType::Arc(arc) => round_z(
      transform.then(Transform3::ocs(&arc.normal)),
      RoundCurve {
        center: Point::new(arc.center.x, arc.center.y),
        radius: arc.radius,
        start: arc.start_angle.to_radians(),
        sweep: sweep(arc.start_angle.to_radians(), arc.end_angle.to_radians()),
        approximate: false,
      },
      arc.center.z,
    ),
    EntityType::LwPolyline(poly) => polyline_z(
      &poly
        .vertices
        .iter()
        .map(|v| (Point::new(v.x, v.y), v.bulge))
        .collect::<Vec<_>>(),
      poly.is_closed(),
      entity.common.elevation,
      transform.then(Transform3::ocs(&poly.extrusion_direction)),
    ),
    EntityType::Polyline(poly) => {
      if poly.flags & (8 | 16 | 64) != 0 {
        ZRange::values(
          poly
            .vertices()
            .filter(|v| v.flags & 128 == 0 || v.flags & 64 != 0)
            .map(|v| transform.z(&v.location)),
        )
      } else {
        polyline_z(
          &poly
            .vertices()
            .map(|v| (Point::new(v.location.x, v.location.y), v.bulge))
            .collect::<Vec<_>>(),
          poly.is_closed(),
          poly.location.z,
          transform.then(Transform3::ocs(&poly.normal)),
        )
      }
    }
    EntityType::Ellipse(ellipse) => {
      let n = &ellipse.normal;
      let a = &ellipse.major_axis;
      let length = n.x.hypot(n.y).hypot(n.z);
      let k = ellipse.minor_axis_ratio / length;
      let b = Vector::new(
        (n.y * a.z - n.z * a.y) * k,
        (n.z * a.x - n.x * a.z) * k,
        (n.x * a.y - n.y * a.x) * k,
      );
      harmonic(
        transform.z(&ellipse.center),
        transform.vector_z(a),
        transform.vector_z(&b),
        ellipse.start_parameter,
        sweep(ellipse.start_parameter, ellipse.end_parameter),
      )
    }
    EntityType::Spline(spline) => {
      // Диапазон контрольных точек консервативен: не скрывает выброс между отсчётами кривой.
      if spline
        .weight_values
        .iter()
        .any(|weight| !weight.is_finite() || *weight <= 0.0)
      {
        return None;
      }
      let source = if spline.control_points.is_empty() {
        &spline.fit_points
      } else {
        &spline.control_points
      };
      let mut range = ZRange::values(source.iter().map(|p| transform.z(p)))?;
      range.approximate = true;
      Some(range)
    }
    EntityType::Face3D(face) => points(
      &[
        &face.first_corner,
        &face.second_corner,
        &face.third_corner,
        &face.fourth_corner,
      ],
      transform,
    ),
    EntityType::Solid(solid) => points(
      &[
        &solid.first_corner,
        &solid.second_corner,
        &solid.third_corner,
        &solid.fourth_corner,
      ],
      transform.then(Transform3::ocs(&solid.extrusion_direction)),
    ),
    EntityType::Trace(trace) => points(
      &[
        &trace.first_corner,
        &trace.second_corner,
        &trace.third_corner,
        &trace.fourth_corner,
      ],
      transform.then(Transform3::ocs(&trace.extrusion_direction)),
    ),
    // Для надписей проверяется точка размещения, а не условный объём шрифта.
    EntityType::Text(text) => {
      let location = if text.horizontal_text_justification as usize != 0
        || text.vertical_text_justification as usize != 0
      {
        &text.second_alignment_point
      } else {
        &text.location
      };
      points(&[location], transform.then(Transform3::ocs(&text.normal)))
    }
    EntityType::MText(text) => points(&[&text.insertion_point], transform),
    EntityType::Attribute(text) => points(
      &[&text.location],
      transform.then(Transform3::ocs(&text.normal)),
    ),
    EntityType::AttributeDefinition(text) => points(
      &[&text.location],
      transform.then(Transform3::ocs(&text.normal)),
    ),
    EntityType::Leader(leader) => ZRange::values(leader.vertices.iter().map(|p| transform.z(p))),
    _ => None,
  }
}

pub fn entity_name(entity: &Entity) -> Option<&'static str> {
  Some(match entity.specific {
    EntityType::Line(_) => "LINE",
    EntityType::ModelPoint(_) => "POINT",
    EntityType::Circle(_) => "CIRCLE",
    EntityType::Arc(_) => "ARC",
    EntityType::LwPolyline(_) => "LWPOLYLINE",
    EntityType::Polyline(_) => "POLYLINE",
    EntityType::Ellipse(_) => "ELLIPSE",
    EntityType::Spline(_) => "SPLINE",
    EntityType::Face3D(_) => "3DFACE",
    EntityType::Solid(_) => "SOLID",
    EntityType::Trace(_) => "TRACE",
    EntityType::Text(_) => "TEXT",
    EntityType::MText(_) => "MTEXT",
    EntityType::Attribute(_) => "ATTRIB",
    EntityType::AttributeDefinition(_) => "ATTDEF",
    EntityType::Leader(_) => "LEADER",
    _ => return None,
  })
}

#[derive(Clone, Debug)]
pub struct Level {
  pub range: ZRange,
  pub count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ElevationReport {
  pub levels: Vec<Level>,
  pub checked: usize,
  pub unknown: usize,
  pub varying: usize,
  pub majority: Option<usize>,
  pub factor: f64,
  pub unit: &'static str,
  pub incomplete: bool,
}

impl ElevationReport {
  pub fn summary(&self) -> String {
    if self.checked == 0 {
      return "Z: нет доступных объектов для проверки".into();
    }
    if self.levels.len() == 1 && self.varying == 0 && self.unknown == 0 && !self.incomplete {
      return format!(
        "Z: все проверенные объекты ({}) на одном уровне {} — в порядке",
        self.checked,
        self.format_range(self.levels[0].range)
      );
    }
    let base = self.majority.map_or_else(
      || "явного большинства нет".to_owned(),
      |index| {
        let level = &self.levels[index];
        format!(
          "большинство: {} ({} из {})",
          self.format_range(level.range),
          level.count,
          self.checked
        )
      },
    );
    format!(
      "Z: уровней {} · {base} · с переменной Z: {} · не определено: {}{}",
      self.levels.len(),
      self.varying,
      self.unknown,
      if self.incomplete {
        " · есть неподдерживаемая геометрия, проверка неполная"
      } else {
        ""
      }
    )
  }

  pub fn format_range(&self, range: ZRange) -> String {
    let (min, max) = (range.min * self.factor, range.max * self.factor);
    if (max - min).abs() < 0.0000005 {
      format!("{} {}", coordinate(min, false), self.unit)
    } else {
      format!(
        "{}…{} {}",
        coordinate(min, false),
        coordinate(max, false),
        self.unit
      )
    }
  }
}

fn coordinate(value: f64, signed: bool) -> String {
  let value = if value.abs() < 0.0000005 { 0.0 } else { value };
  let value = if signed {
    format!("{value:+.6}")
  } else {
    format!("{value:.6}")
  };
  value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

pub fn analyze(item: &DrawingItem, report: &mut DiagnosticReport) {
  let tolerance = TOLERANCE_MM / item.units.factor();
  let visible: Vec<_> = item
    .appearance
    .elevations
    .iter()
    .enumerate()
    .filter(|(_, object)| item.appearance.visible(&object.style))
    .collect();
  let mut result = ElevationReport {
    checked: visible.len(),
    factor: item.units.factor(),
    unit: item.units.label(),
    incomplete: item.unsupported_entities > 0,
    ..Default::default()
  };
  let mut flat: Vec<_> = visible
    .iter()
    .filter_map(|(index, object)| {
      let range = object.range?;
      (range.max - range.min <= tolerance).then_some((*index, range))
    })
    .collect();
  flat.sort_by(|a, b| a.1.min.total_cmp(&b.1.min));
  // Сначала находим наиболее населённый интервал: одиночный нижний выброс
  // не должен разрезать большинство на границе допуска.
  let mut maximums = std::collections::VecDeque::<usize>::new();
  let mut left = 0;
  let mut best = 0..0;
  for right in 0..flat.len() {
    while maximums
      .back()
      .is_some_and(|&i| flat[i].1.max <= flat[right].1.max)
    {
      maximums.pop_back();
    }
    maximums.push_back(right);
    while maximums
      .front()
      .is_some_and(|&i| flat[i].1.max - flat[left].1.min > tolerance)
    {
      if maximums.front() == Some(&left) {
        maximums.pop_front();
      }
      left += 1;
    }
    if right + 1 - left > best.len() {
      best = left..right + 1;
    }
  }
  let majority_members: std::collections::HashSet<_> = if best.len() > result.checked / 2 {
    flat[best].iter().map(|(index, _)| *index).collect()
  } else {
    Default::default()
  };
  let mut groups: Vec<(Level, Vec<usize>)> = Vec::new();
  if !majority_members.is_empty() {
    let range = ZRange::values(
      flat
        .iter()
        .filter(|(index, _)| majority_members.contains(index))
        .flat_map(|(_, range)| [range.min, range.max]),
    )
    .unwrap();
    groups.push((
      Level {
        range,
        count: majority_members.len(),
      },
      majority_members.iter().copied().collect(),
    ));
  }
  let mut other_group: Option<usize> = None;
  for (index, range) in flat {
    if majority_members.contains(&index) {
      continue;
    }
    if other_group
      .is_none_or(|i| range.max.max(groups[i].0.range.max) - groups[i].0.range.min > tolerance)
    {
      other_group = Some(groups.len());
      groups.push((Level { range, count: 0 }, Vec::new()));
    }
    let (level, members) = &mut groups[other_group.unwrap()];
    level.count += 1;
    level.range.max = level.range.max.max(range.max);
    members.push(index);
  }
  groups.sort_by(|a, b| a.0.range.min.total_cmp(&b.0.range.min));
  let mut membership = vec![None; item.appearance.elevations.len()];
  for (group, (level, members)) in groups.into_iter().enumerate() {
    for index in members {
      membership[index] = Some(group);
    }
    result.levels.push(level);
  }
  result.majority = result
    .levels
    .iter()
    .position(|level| level.count > result.checked / 2);
  for (index, object) in visible {
    if result
      .majority
      .is_some_and(|majority| membership[index] == Some(majority))
    {
      continue;
    }
    let marker = if object.primitives.is_empty() {
      Marker::Bounds(object.bounds)
    } else {
      Marker::Contour(object.primitives.clone().collect())
    };
    let layer = item
      .appearance
      .layers
      .get(object.style.layer)
      .map_or("0", |layer| layer.name.as_str());
    let identity = format!("Объект {}: {}, слой «{}»", index + 1, object.source, layer);
    let Some(range) = object.range else {
      result.unknown += 1;
      report.add(
        IssueKind::ZLevel,
        marker,
        format!("{identity}. Z не определена: проверка высоты неполная."),
      );
      continue;
    };
    let varying = range.max - range.min > tolerance;
    result.varying += usize::from(varying);
    let difference = if let Some(majority) = result.majority {
      let reference = result.levels[majority].range.center();
      let delta_min = coordinate((range.min - reference) * result.factor, true);
      let delta_max = coordinate((range.max - reference) * result.factor, true);
      let delta = if delta_min == delta_max {
        delta_min
      } else {
        format!("{delta_min}…{delta_max}")
      };
      format!(
        "Основной уровень {} {}; ΔZ {delta} {}.",
        coordinate(reference * result.factor, false),
        result.unit,
        result.unit
      )
    } else {
      "Единого уровня большинства нет; сравните уровни в сводке.".into()
    };
    let note = if range.approximate {
      " Диапазон оценочный: по контрольным/опорным точкам кривой."
    } else {
      ""
    };
    report.add(
      IssueKind::ZLevel,
      marker,
      format!(
        "{identity}. Z = {}. {}{difference}{note}",
        result.format_range(range),
        if varying {
          "Объект не лежит на одном уровне Z. "
        } else {
          ""
        }
      ),
    );
  }
  report.elevation = result;
}
