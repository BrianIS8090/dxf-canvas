use std::path::{Path, PathBuf};

use dxf::{
  Drawing,
  entities::{Entity, EntityType},
};
use thiserror::Error;

use crate::geometry::{
  Bounds, DrawingItem, LengthUnit, MeasureCurve, Point, Primitive, RoundCurve,
};

const CURVE_SEGMENTS: usize = 72;
const MAX_RECURSION_DEPTH: usize = 12;

#[derive(Debug, Error)]
pub enum ImportError {
  #[error("не удалось прочитать данные DXF: {0}")]
  Supplemental(#[from] std::io::Error),
  #[error("не удалось прочитать DXF: {0}")]
  Read(#[from] dxf::DxfError),
  #[error("в файле нет поддерживаемой двумерной геометрии")]
  NoGeometry,
  #[error("не удалось определить имя файла")]
  MissingFileName,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Transform2 {
  a: f64,
  b: f64,
  c: f64,
  d: f64,
  tx: f64,
  ty: f64,
}

impl Transform2 {
  pub(crate) const IDENTITY: Self = Self {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    tx: 0.0,
    ty: 0.0,
  };

  pub(crate) fn apply(self, point: Point) -> Point {
    Point::new(
      self.a * point.x + self.c * point.y + self.tx,
      self.b * point.x + self.d * point.y + self.ty,
    )
  }

  pub(crate) fn then(self, child: Self) -> Self {
    Self {
      a: self.a * child.a + self.c * child.b,
      b: self.b * child.a + self.d * child.b,
      c: self.a * child.c + self.c * child.d,
      d: self.b * child.c + self.d * child.d,
      tx: self.a * child.tx + self.c * child.ty + self.tx,
      ty: self.b * child.tx + self.d * child.ty + self.ty,
    }
  }

  pub(crate) fn ocs(normal: &dxf::Vector, elevation: f64) -> Self {
    let length = normal.x.hypot(normal.y).hypot(normal.z);
    if !length.is_finite() || length < 1.0e-12 {
      return Self::IDENTITY;
    }
    let (nx, ny, nz) = (normal.x / length, normal.y / length, normal.z / length);
    // Оси OCS строятся по алгоритму произвольной оси DXF, включая нормаль -Z.
    let (ax, ay, az) = if nx.abs() < 1.0 / 64.0 && ny.abs() < 1.0 / 64.0 {
      (nz, 0.0, -nx)
    } else {
      (-ny, nx, 0.0)
    };
    let axis_length = ax.hypot(ay).hypot(az);
    let (ax, ay, az) = (ax / axis_length, ay / axis_length, az / axis_length);
    Self {
      a: ax,
      b: ay,
      c: ny * az - nz * ay,
      d: nz * ax - nx * az,
      tx: nx * elevation,
      ty: ny * elevation,
    }
  }

  fn round(self, curve: RoundCurve) -> Option<RoundCurve> {
    let sx = self.a.hypot(self.b);
    let sy = self.c.hypot(self.d);
    let dot = self.a * self.c + self.b * self.d;
    if sx < 1.0e-12 || (sx - sy).abs() > sx * 1.0e-9 || dot.abs() > sx * sy * 1.0e-9 {
      return None;
    }
    let center = self.apply(curve.center);
    let start_point = self.apply(curve.point_at(curve.start));
    Some(RoundCurve {
      center,
      radius: curve.radius * sx,
      start: (start_point.y - center.y).atan2(start_point.x - center.x),
      sweep: curve.sweep * (self.a * self.d - self.b * self.c).signum(),
      approximate: curve.approximate,
    })
  }

  pub(crate) fn insert(
    location: Point,
    base: Point,
    scale_x: f64,
    scale_y: f64,
    rotation_degrees: f64,
  ) -> Self {
    let angle = rotation_degrees.to_radians();
    let (sin, cos) = angle.sin_cos();
    let a = cos * scale_x;
    let b = sin * scale_x;
    let c = -sin * scale_y;
    let d = cos * scale_y;
    Self {
      a,
      b,
      c,
      d,
      tx: location.x - a * base.x - c * base.y,
      ty: location.y - b * base.x - d * base.y,
    }
  }
}

pub fn load_dxf(path: &Path) -> Result<DrawingItem, ImportError> {
  // Обе части импорта используют одни байты и одну кодировку, выбранную по заголовку.
  let bytes = std::fs::read(path)?;
  let encoding = crate::raw_dxf::text_encoding(&bytes);
  let drawing = Drawing::load_with_encoding(&mut bytes.as_slice(), encoding)?;
  let raw = crate::raw_dxf::RawDxf::from_bytes(&bytes, encoding);
  drop(bytes);
  let (primitives, appearance, unsupported_entities) = crate::dxf_scene::extract(&drawing, &raw);
  let mut bounds = Bounds::empty();
  for primitive in &primitives {
    if let Some(primitive_bounds) = primitive.bounds() {
      bounds.include_bounds(primitive_bounds);
    }
  }
  for text in &appearance.texts {
    bounds.include_bounds(text.bounds);
  }
  for fill in &appearance.fills {
    bounds.include_bounds(fill.bounds);
  }

  if !bounds.is_valid() {
    return Err(ImportError::NoGeometry);
  }
  bounds = padded_degenerate_bounds(bounds);

  let name = path
    .file_stem()
    .and_then(|name| name.to_str())
    .filter(|name| !name.is_empty())
    .ok_or(ImportError::MissingFileName)?
    .to_owned();

  Ok(DrawingItem {
    appearance,
    units: LengthUnit::from_dxf_code(drawing.header.default_drawing_units as i16),
    path: PathBuf::from(path),
    name,
    primitives,
    bounds,
    offset: Point::default(),
    scale: 1.0,
    unsupported_entities,
  })
}

#[cfg(test)]
fn extract_primitives(drawing: &Drawing) -> (Vec<Primitive>, usize) {
  let (primitives, _, unsupported) = crate::dxf_scene::extract(drawing, &Default::default());
  (primitives, unsupported)
}

pub(crate) fn append_entity(
  drawing: &Drawing,
  entity: &Entity,
  transform: Transform2,
  depth: usize,
  output: &mut Vec<Primitive>,
  unsupported: &mut usize,
) {
  match &entity.specific {
    EntityType::Line(line) => push_path(
      output,
      vec![point(line.p1.x, line.p1.y), point(line.p2.x, line.p2.y)],
      false,
      transform,
    ),
    EntityType::Circle(circle) => {
      push_round(
        output,
        RoundCurve {
          center: point(circle.center.x, circle.center.y),
          radius: circle.radius,
          start: 0.0,
          sweep: std::f64::consts::TAU,
          approximate: false,
        },
        transform.then(Transform2::ocs(&circle.normal, circle.center.z)),
      );
    }
    EntityType::Arc(arc) => {
      let (start, sweep) = degree_sweep(arc.start_angle, arc.end_angle);
      push_round(
        output,
        RoundCurve {
          center: point(arc.center.x, arc.center.y),
          radius: arc.radius,
          start,
          sweep,
          approximate: false,
        },
        transform.then(Transform2::ocs(&arc.normal, arc.center.z)),
      );
    }
    EntityType::Ellipse(ellipse) => {
      let major = point(ellipse.major_axis.x, ellipse.major_axis.y);
      let minor = point(
        -major.y * ellipse.minor_axis_ratio,
        major.x * ellipse.minor_axis_ratio,
      );
      let sweep = positive_sweep(ellipse.start_parameter, ellipse.end_parameter);
      let closed = (sweep - std::f64::consts::TAU).abs() < 0.0001;
      let points = sample_parametric(
        ellipse.start_parameter,
        ellipse.start_parameter + sweep,
        curve_segment_count(sweep),
        |angle| {
          Point::new(
            ellipse.center.x + major.x * angle.cos() + minor.x * angle.sin(),
            ellipse.center.y + major.y * angle.cos() + minor.y * angle.sin(),
          )
        },
      );
      push_path(output, points, closed, transform);
    }
    EntityType::LwPolyline(polyline) => {
      let vertices: Vec<_> = polyline
        .vertices
        .iter()
        .map(|vertex| (Point::new(vertex.x, vertex.y), vertex.bulge))
        .collect();
      let combined = transform.then(Transform2::ocs(
        &polyline.extrusion_direction,
        entity.common.elevation,
      ));
      push_bulged_polyline(output, &vertices, polyline.is_closed(), combined);
    }
    EntityType::Polyline(polyline) => {
      let vertices: Vec<_> = polyline
        .vertices()
        .map(|vertex| {
          (
            Point::new(vertex.location.x, vertex.location.y),
            vertex.bulge,
          )
        })
        .collect();
      // Вершины 3D-полилиний уже находятся в мировой системе координат.
      let combined = if polyline.flags & (8 | 16 | 64) == 0 {
        transform.then(Transform2::ocs(&polyline.normal, polyline.location.z))
      } else {
        transform
      };
      push_bulged_polyline(output, &vertices, polyline.is_closed(), combined);
    }
    EntityType::Spline(spline) => {
      let points = sample_spline(spline);
      let new_index = output.len();
      push_path(output, points, spline.is_closed(), transform);
      if let Some(Primitive::Path { points, curves, .. }) = output.get_mut(new_index)
        && let Some(curve) = recognize_circular_spline(points)
      {
        *curves = vec![MeasureCurve::Round(curve)];
      }
    }
    EntityType::Solid(solid) => {
      let points = [
        &solid.first_corner,
        &solid.second_corner,
        &solid.fourth_corner,
        &solid.third_corner,
      ]
      .into_iter()
      .map(|corner| {
        Transform2::ocs(&solid.extrusion_direction, corner.z).apply(point(corner.x, corner.y))
      })
      .collect();
      push_path(output, points, true, transform);
    }
    EntityType::Face3D(face) => push_path(
      output,
      vec![
        point(face.first_corner.x, face.first_corner.y),
        point(face.second_corner.x, face.second_corner.y),
        point(face.third_corner.x, face.third_corner.y),
        point(face.fourth_corner.x, face.fourth_corner.y),
      ],
      true,
      transform,
    ),
    EntityType::ModelPoint(model_point) => output.push(Primitive::Point(
      transform.apply(point(model_point.location.x, model_point.location.y)),
    )),
    EntityType::Insert(insert) if depth < MAX_RECURSION_DEPTH => {
      if let Some(block) = drawing.blocks().find(|block| block.name == insert.name) {
        let rows = insert.row_count.max(1) as usize;
        let columns = insert.column_count.max(1) as usize;
        for row in 0..rows {
          for column in 0..columns {
            let location = Point::new(
              insert.location.x + column as f64 * insert.column_spacing,
              insert.location.y + row as f64 * insert.row_spacing,
            );
            let insert_transform = Transform2::insert(
              location,
              point(block.base_point.x, block.base_point.y),
              insert.x_scale_factor,
              insert.y_scale_factor,
              insert.rotation,
            );
            let combined = transform
              .then(Transform2::ocs(
                &insert.extrusion_direction,
                insert.location.z,
              ))
              .then(insert_transform);
            for block_entity in &block.entities {
              append_entity(
                drawing,
                block_entity,
                combined,
                depth + 1,
                output,
                unsupported,
              );
            }
          }
        }
      } else {
        *unsupported += 1;
      }
    }
    _ => *unsupported += 1,
  }
}

fn push_bulged_polyline(
  output: &mut Vec<Primitive>,
  vertices: &[(Point, f64)],
  closed: bool,
  transform: Transform2,
) {
  if vertices.len() < 2 {
    if let Some((point, _)) = vertices.first() {
      output.push(Primitive::Point(transform.apply(*point)));
    }
    return;
  }

  let segment_count = if closed {
    vertices.len()
  } else {
    vertices.len() - 1
  };
  let mut points = Vec::new();
  let mut measure_curves = Vec::new();
  for index in 0..segment_count {
    let (start, bulge) = vertices[index];
    let end = vertices[(index + 1) % vertices.len()].0;
    let mut segment = sample_bulge(start, end, bulge);
    let curve = if bulge.abs() < 1.0e-10 {
      MeasureCurve::Line {
        start: transform.apply(start),
        end: transform.apply(end),
      }
    } else if let Some(curve) =
      bulge_round(start, end, bulge).and_then(|curve| transform.round(curve))
    {
      MeasureCurve::Round(curve)
    } else {
      MeasureCurve::Polyline {
        points: segment
          .iter()
          .map(|point| transform.apply(*point))
          .collect(),
        closed: false,
      }
    };
    measure_curves.push(curve);
    if index > 0 && !segment.is_empty() {
      segment.remove(0);
    }
    points.extend(segment);
  }
  push_path(output, points, closed, transform);
  if let Some(Primitive::Path { curves, .. }) = output.last_mut() {
    *curves = measure_curves;
  }
}

fn bulge_round(start: Point, end: Point, bulge: f64) -> Option<RoundCurve> {
  let dx = end.x - start.x;
  let dy = end.y - start.y;
  let chord = dx.hypot(dy);
  if chord < 1.0e-10 || bulge.abs() < 1.0e-10 {
    return None;
  }
  let offset = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
  let center = point(
    (start.x + end.x) * 0.5 - dy / chord * offset,
    (start.y + end.y) * 0.5 + dx / chord * offset,
  );
  Some(RoundCurve {
    center,
    radius: chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs()),
    start: (start.y - center.y).atan2(start.x - center.x),
    sweep: 4.0 * bulge.atan(),
    approximate: false,
  })
}

pub(crate) fn sample_bulge(start: Point, end: Point, bulge: f64) -> Vec<Point> {
  if bulge.abs() < 1.0e-10 {
    return vec![start, end];
  }

  let dx = end.x - start.x;
  let dy = end.y - start.y;
  let chord = dx.hypot(dy);
  if chord < 1.0e-10 {
    return vec![start];
  }

  let center_offset = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
  let midpoint = Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
  let center = Point::new(
    midpoint.x - dy / chord * center_offset,
    midpoint.y + dx / chord * center_offset,
  );
  let start_angle = (start.y - center.y).atan2(start.x - center.x);
  let sweep = 4.0 * bulge.atan();
  sample_parametric(
    start_angle,
    start_angle + sweep,
    curve_segment_count(sweep.abs()),
    |angle| {
      let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
      Point::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
      )
    },
  )
}

pub(crate) fn sample_spline(spline: &dxf::entities::Spline) -> Vec<Point> {
  if spline.control_points.len() >= 2 {
    let degree =
      (spline.degree_of_curve.max(1) as usize).min(spline.control_points.len().saturating_sub(1));
    let point_count = spline.control_points.len();
    if spline.knot_values.len() > point_count + degree {
      let start = spline.knot_values[degree];
      let end = spline.knot_values[point_count];
      if end > start {
        let segments = (point_count * 12).clamp(CURVE_SEGMENTS, 480);
        return (0..=segments)
          .filter_map(|index| {
            let ratio = index as f64 / segments as f64;
            let parameter = if index == segments {
              end
            } else {
              start + (end - start) * ratio
            };
            evaluate_spline(spline, degree, parameter)
          })
          .collect();
      }
    }
  }

  let source = if spline.fit_points.len() >= 2 {
    &spline.fit_points
  } else {
    &spline.control_points
  };
  source
    .iter()
    .map(|point| Point::new(point.x, point.y))
    .collect()
}

fn evaluate_spline(spline: &dxf::entities::Spline, degree: usize, parameter: f64) -> Option<Point> {
  let count = spline.control_points.len();
  let knots = &spline.knot_values;
  let last_span = count.saturating_sub(1);
  let span = if (parameter - knots[count]).abs() < 1.0e-10 {
    last_span
  } else {
    (degree..count).find(|index| parameter >= knots[*index] && parameter < knots[*index + 1])?
  };

  let mut working = Vec::with_capacity(degree + 1);
  for local in 0..=degree {
    let index = span - degree + local;
    let source = &spline.control_points[index];
    let weight = spline.weight_values.get(index).copied().unwrap_or(1.0);
    working.push((source.x * weight, source.y * weight, weight));
  }

  for level in 1..=degree {
    for local in (level..=degree).rev() {
      let index = span - degree + local;
      let denominator = knots[index + degree - level + 1] - knots[index];
      let alpha = if denominator.abs() < 1.0e-12 {
        0.0
      } else {
        (parameter - knots[index]) / denominator
      };
      let previous = working[local - 1];
      let current = working[local];
      working[local] = (
        previous.0 * (1.0 - alpha) + current.0 * alpha,
        previous.1 * (1.0 - alpha) + current.1 * alpha,
        previous.2 * (1.0 - alpha) + current.2 * alpha,
      );
    }
  }

  let result = working[degree];
  (result.2.abs() > 1.0e-12).then(|| Point::new(result.0 / result.2, result.1 / result.2))
}

fn push_path(output: &mut Vec<Primitive>, points: Vec<Point>, closed: bool, transform: Transform2) {
  let points: Vec<_> = points
    .into_iter()
    .map(|point| transform.apply(point))
    .collect();
  if points.len() >= 2 {
    let curves = if points.len() == 2 {
      vec![MeasureCurve::Line {
        start: points[0],
        end: points[1],
      }]
    } else {
      vec![MeasureCurve::Polyline {
        points: points.clone(),
        closed,
      }]
    };
    output.push(Primitive::Path {
      points,
      closed,
      curves,
    });
  } else if let Some(point) = points.first() {
    output.push(Primitive::Point(*point));
  }
}

fn push_round(output: &mut Vec<Primitive>, curve: RoundCurve, transform: Transform2) {
  let points = sample_parametric(
    curve.start,
    curve.start + curve.sweep,
    curve_segment_count(curve.sweep),
    |angle| curve.point_at(angle),
  );
  push_path(output, points, curve.is_full(), transform);
  if let (Some(curve), Some(Primitive::Path { curves, .. })) =
    (transform.round(curve), output.last_mut())
  {
    *curves = vec![MeasureCurve::Round(curve)];
  }
}

fn recognize_circular_spline(points: &[Point]) -> Option<RoundCurve> {
  if points.len() < 9 {
    return None;
  }
  let origin = points[0];
  let distance = |a: Point, b: Point| (a.x - b.x).hypot(a.y - b.y);
  let farthest = *points
    .iter()
    .max_by(|a, b| distance(**a, origin).total_cmp(&distance(**b, origin)))?;
  let v = point(farthest.x - origin.x, farthest.y - origin.y);
  let cross = |p: Point| v.x * (p.y - origin.y) - v.y * (p.x - origin.x);
  let third = *points
    .iter()
    .max_by(|a, b| cross(**a).abs().total_cmp(&cross(**b).abs()))?;
  let w = point(third.x - origin.x, third.y - origin.y);
  let determinant = 2.0 * (v.x * w.y - v.y * w.x);
  if determinant.abs() < 1.0e-10 {
    return None;
  }
  let vv = v.x * v.x + v.y * v.y;
  let ww = w.x * w.x + w.y * w.y;
  let center = point(
    origin.x + (vv * w.y - ww * v.y) / determinant,
    origin.y + (v.x * ww - w.x * vv) / determinant,
  );
  let radius = distance(origin, center);
  if !radius.is_finite() || radius < 1.0e-9 {
    return None;
  }
  let tolerance = (radius * 1.0e-4).max(1.0e-6);
  if points
    .iter()
    .any(|p| (distance(*p, center) - radius).abs() > tolerance)
  {
    return None;
  }
  let angle = |p: Point| (p.y - center.y).atan2(p.x - center.x);
  let mut sweep: f64 = 0.0;
  for pair in points.windows(2) {
    let delta = (angle(pair[1]) - angle(pair[0]) + std::f64::consts::PI)
      .rem_euclid(std::f64::consts::TAU)
      - std::f64::consts::PI;
    if delta.abs() > 1.0e-9 && sweep.abs() > 1.0e-9 && delta.signum() != sweep.signum() {
      return None;
    }
    sweep += delta;
  }
  if sweep.abs() < 0.05 || sweep.abs() > std::f64::consts::TAU + 1.0e-6 {
    return None;
  }
  if distance(origin, *points.last()?) <= tolerance && sweep.abs() > 6.2 {
    sweep = std::f64::consts::TAU.copysign(sweep);
  }
  Some(RoundCurve {
    center,
    radius,
    start: angle(origin),
    sweep,
    approximate: true,
  })
}

fn sample_parametric(
  start: f64,
  end: f64,
  segments: usize,
  sample: impl Fn(f64) -> Point,
) -> Vec<Point> {
  let segments = segments.max(1);
  (0..=segments)
    .map(|index| {
      let ratio = index as f64 / segments as f64;
      sample(start + (end - start) * ratio)
    })
    .collect()
}

fn curve_segment_count(sweep: f64) -> usize {
  // Погрешность последнего разряда после DWG → DXF не должна менять число точек полукруга.
  ((sweep.abs() / std::f64::consts::TAU * CURVE_SEGMENTS as f64 - 1.0e-10).ceil() as usize)
    .clamp(4, CURVE_SEGMENTS * 4)
}

fn degree_sweep(start_degrees: f64, end_degrees: f64) -> (f64, f64) {
  let start = start_degrees.to_radians();
  let end = end_degrees.to_radians();
  (start, positive_sweep(start, end))
}

fn positive_sweep(start: f64, end: f64) -> f64 {
  let mut sweep = end - start;
  while sweep <= 0.0 {
    sweep += std::f64::consts::TAU;
  }
  sweep.min(std::f64::consts::TAU)
}

fn padded_degenerate_bounds(mut bounds: Bounds) -> Bounds {
  if bounds.width() < 1.0e-9 {
    bounds.min.x -= 0.5;
    bounds.max.x += 0.5;
  }
  if bounds.height() < 1.0e-9 {
    bounds.min.y -= 0.5;
    bounds.max.y += 0.5;
  }
  bounds
}

const fn point(x: f64, y: f64) -> Point {
  Point::new(x, y)
}

#[cfg(test)]
mod tests {
  use std::time::{SystemTime, UNIX_EPOCH};

  use dxf::{
    Block, LwPolylineVertex, Point as DxfPoint, Vector,
    entities::{Arc, Circle, Entity, Insert, Line, LwPolyline, Polyline, Vertex},
  };

  use super::*;

  #[test]
  fn legacy_dxf_keeps_cyrillic_layer_names() {
    let source = concat!(
      "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1018\n9\n$DWGCODEPAGE\n3\nansi_1251\n0\nENDSEC\n",
      "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n1\n0\nLAYER\n2\nНовый_Стены\n70\n0\n62\n3\n6\nCONTINUOUS\n0\nENDTAB\n0\nENDSEC\n",
      "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nНовый_Стены\n10\n0\n20\n0\n11\n10\n21\n5\n0\nENDSEC\n0\nEOF\n"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("кириллица.dxf");
    let (bytes, _, errors) = encoding_rs::WINDOWS_1251.encode(source);
    assert!(!errors);
    std::fs::write(&path, bytes.as_ref()).unwrap();
    let item = load_dxf(&path).unwrap();
    let layer = &item.appearance.layers[item.appearance.styles[0].layer];
    assert_eq!(layer.name, "Новый_Стены");
    assert_eq!(std::fs::read(&path).unwrap(), bytes.as_ref());
  }

  #[test]
  fn header_encoding_keeps_layers_blocks_text_and_visibility_in_sync() {
    for (version, page, encoding, name, text) in [
      (
        "AC1018",
        "ansi_1251",
        encoding_rs::WINDOWS_1251,
        "Новый_Стены",
        "План этажа № 1",
      ),
      (
        "AC1018",
        "ANSI_1251",
        encoding_rs::WINDOWS_1251,
        "Новый_Стены",
        "План этажа № 1",
      ),
      (
        "AC1018",
        "ANSI_1252",
        encoding_rs::WINDOWS_1252,
        "Büro_façade",
        "Maß 10 m²",
      ),
      (
        "AC1032",
        "ANSI_1251",
        encoding_rs::UTF_8,
        "Новый_Стены",
        "План этажа № 1",
      ),
    ] {
      let source = format!(
        concat!(
          "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\n{}\n9\n$DWGCODEPAGE\n3\n{}\n0\nENDSEC\n",
          "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n1\n0\nLAYER\n2\n{}\n70\n1\n62\n3\n6\nCONTINUOUS\n0\nENDTAB\n0\nENDSEC\n",
          "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nBlock_{}\n70\n0\n10\n0\n20\n0\n30\n0\n",
          "0\nLINE\n8\n{}\n10\n0\n20\n0\n11\n10\n21\n5\n",
          "0\nMTEXT\n8\n{}\n10\n0\n20\n0\n40\n1\n1\n{}\n0\nENDBLK\n0\nENDSEC\n",
          "0\nSECTION\n2\nENTITIES\n0\nINSERT\n8\n{}\n2\nBlock_{}\n10\n0\n20\n0\n0\nENDSEC\n0\nEOF\n"
        ),
        version, page, name, name, name, name, text, name, name
      );
      let (bytes, _, errors) = encoding.encode(&source);
      assert!(!errors);
      let directory = tempfile::tempdir().unwrap();
      let path = directory.path().join("encoding.dxf");
      std::fs::write(&path, bytes.as_ref()).unwrap();
      let item = load_dxf(&path).unwrap();
      assert_eq!(item.unsupported_entities, 0);
      assert_eq!(item.primitives.len(), 1);
      let style = &item.appearance.styles[0];
      let layer = &item.appearance.layers[style.layer];
      assert_eq!(layer.name, name, "{version}/{page}");
      assert!(
        !layer.initial_visible,
        "Данные заморозки должны относиться к тому же слою"
      );
      assert_eq!(style.color, eframe::egui::Color32::GREEN);
      assert_eq!(item.appearance.texts[0].text, text, "{version}/{page}");
      assert_eq!(item.appearance.texts[0].style.layer, style.layer);
    }
  }

  #[test]
  fn equivalent_half_circles_have_the_same_display_sampling() {
    let semicircle = std::f64::consts::PI;
    assert_eq!(curve_segment_count(semicircle), 36);
    assert_eq!(curve_segment_count(semicircle + 1.0e-15), 36);
    assert_eq!(curve_segment_count(-semicircle - 1.0e-15), 36);
    assert_eq!(curve_segment_count(semicircle + 0.01), 37);
  }

  #[test]
  fn multiline_text_is_not_discarded() {
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::MText(dxf::entities::MText {
      text: "Потолок\\PЭтаж 1".to_owned(),
      insertion_point: DxfPoint::new(12.0, 30.0, 0.0),
      initial_text_height: 2.5,
      ..Default::default()
    })));
    let (_, appearance, unsupported) = crate::dxf_scene::extract(&drawing, &Default::default());
    assert_eq!(appearance.texts.len(), 1);
    assert_eq!(appearance.texts[0].origin, Point::new(12.0, 30.0));
    assert_eq!(
      crate::cad_text::plain(&appearance.texts[0].text),
      "Потолок\nЭтаж 1"
    );
    assert_eq!(
      unsupported, 0,
      "MTEXT должен отображаться, а не пропускаться"
    );
  }

  #[test]
  #[ignore = "Нужен локальный эталон DXF_REFERENCE_FIXTURE, чертёж не публикуется"]
  fn reference_drawing_has_no_missing_entities() {
    let path = std::env::var_os("DXF_REFERENCE_FIXTURE").expect("Задайте DXF_REFERENCE_FIXTURE");
    let started = std::time::Instant::now();
    let item = load_dxf(Path::new(&path)).unwrap();
    eprintln!(
      "Эталон: {} элементов, {} пропущено, {:?}",
      item.primitives.len(),
      item.unsupported_entities,
      started.elapsed()
    );
    eprintln!(
      "Слои: {}, тексты: {}, заливки: {}, предупреждения: {:?}",
      item.appearance.layers.len(),
      item.appearance.texts.len(),
      item.appearance.fills.len(),
      item.appearance.warnings
    );
    assert_eq!(
      item.unsupported_entities, 0,
      "Часть эталонного чертежа не отображается"
    );
    assert!(item.appearance.warnings.is_empty());
    assert_eq!(item.appearance.layers.len(), 168);
    assert_eq!(item.appearance.source_counts["HATCH"], 566);
    assert_eq!(item.appearance.source_counts["ARC_DIMENSION"], 7);
    assert_eq!(item.appearance.texts.len(), 337);
    assert_eq!(item.appearance.fills.len(), 735);
    assert!(item.primitives.len() > 110_000);
    assert_eq!(item.appearance.styles.len(), item.primitives.len());
    assert!(item.appearance.fills.iter().all(|fill| {
      !fill.indices.is_empty()
        && fill
          .indices
          .iter()
          .all(|index| (*index as usize) < fill.vertices.len())
    }));
  }

  #[test]
  fn semicircle_bulge_passes_through_expected_height() {
    let points = sample_bulge(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 1.0);
    let bounds = Bounds::from_points(points).unwrap();
    assert!((bounds.width() - 10.0).abs() < 0.001);
    assert!((bounds.height() - 5.0).abs() < 0.01);
  }

  #[test]
  fn insert_transform_respects_base_point_rotation_and_scale() {
    let transform = Transform2::insert(
      Point::new(100.0, 50.0),
      Point::new(10.0, 0.0),
      2.0,
      3.0,
      90.0,
    );
    let result = transform.apply(Point::new(10.0, 0.0));
    assert!((result.x - 100.0).abs() < 0.001);
    assert!((result.y - 50.0).abs() < 0.001);
  }

  #[test]
  fn saved_dxf_is_loaded_with_real_bounds_and_file_name() {
    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_nanos();
    let path = std::env::temp_dir().join(format!("dxf_canvas_{unique}.dxf"));
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::Line(Line::new(
      DxfPoint::new(-20.0, 10.0, 0.0),
      DxfPoint::new(80.0, 60.0, 0.0),
    ))));
    drawing.save_file(&path).unwrap();

    let item = load_dxf(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(item.path, path);
    assert!(item.name.starts_with("dxf_canvas_"));
    assert!((item.bounds.width() - 100.0).abs() < 0.001);
    assert!((item.bounds.height() - 50.0).abs() < 0.001);
    assert_eq!(item.primitives.len(), 1);
  }

  #[test]
  fn circle_with_reversed_normal_is_converted_from_ocs() {
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::Circle(Circle {
      center: DxfPoint::new(40.0, 20.0, 0.0),
      radius: 5.0,
      normal: Vector::new(0.0, 0.0, -1.0),
      ..Default::default()
    })));

    let (primitives, unsupported) = extract_primitives(&drawing);
    let bounds = primitives[0].bounds().unwrap();

    assert_eq!(unsupported, 0);
    assert!((bounds.min.x + 45.0).abs() < 0.001);
    assert!((bounds.max.x + 35.0).abs() < 0.001);
    assert!((bounds.min.y - 15.0).abs() < 0.001);
    assert!((bounds.max.y - 25.0).abs() < 0.001);
  }

  #[test]
  fn arc_with_reversed_normal_preserves_ocs_angles() {
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::Arc(Arc {
      center: DxfPoint::new(10.0, 20.0, 0.0),
      radius: 5.0,
      normal: Vector::new(0.0, 0.0, -1.0),
      start_angle: 0.0,
      end_angle: 90.0,
      ..Default::default()
    })));
    let (primitives, _) = extract_primitives(&drawing);
    let Primitive::Path { points, .. } = &primitives[0] else {
      panic!()
    };
    assert_eq!(points.first().copied(), Some(Point::new(-15.0, 20.0)));
    assert_eq!(points.last().copied(), Some(Point::new(-10.0, 25.0)));
  }

  #[test]
  fn lwpolyline_bulges_are_transformed_after_sampling() {
    let mut drawing = Drawing::new();
    drawing.add_entity(Entity::new(EntityType::LwPolyline(LwPolyline {
      extrusion_direction: Vector::new(0.0, 0.0, -1.0),
      vertices: vec![
        LwPolylineVertex {
          x: 0.0,
          y: 0.0,
          bulge: 1.0,
          ..Default::default()
        },
        LwPolylineVertex {
          x: 10.0,
          y: 0.0,
          ..Default::default()
        },
      ],
      ..Default::default()
    })));
    let (primitives, _) = extract_primitives(&drawing);
    let bounds = primitives[0].bounds().unwrap();
    assert!((bounds.min.x + 10.0).abs() < 0.001);
    assert!(bounds.max.x.abs() < 0.001);
    assert!((bounds.min.y + 5.0).abs() < 0.001);
  }

  #[test]
  fn polyline_2d_uses_ocs_but_polyline_3d_keeps_world_coordinates() {
    for is_3d in [false, true] {
      let mut drawing = Drawing::new();
      let mut polyline = Polyline {
        normal: Vector::new(0.0, 0.0, -1.0),
        ..Default::default()
      };
      polyline.set_is_3d_polyline(is_3d);
      for x in [10.0, 20.0] {
        polyline.add_vertex(
          &mut drawing,
          Vertex {
            location: DxfPoint::new(x, 5.0, 0.0),
            ..Default::default()
          },
        );
      }
      drawing.add_entity(Entity::new(EntityType::Polyline(polyline)));
      let (primitives, _) = extract_primitives(&drawing);
      let bounds = primitives[0].bounds().unwrap();
      assert_eq!(bounds.min.x, if is_3d { 10.0 } else { -20.0 });
      assert_eq!(bounds.max.x, if is_3d { 20.0 } else { -10.0 });
    }
  }

  #[test]
  fn ocs_accounts_for_elevation_and_normalizes_normal() {
    let projected = Transform2::ocs(&Vector::new(0.0, 2.0, 0.0), 7.0).apply(Point::new(10.0, 20.0));
    assert_eq!(projected, Point::new(-10.0, 7.0));
  }

  #[test]
  fn insert_and_child_ocs_are_composed_in_the_correct_order() {
    let mut drawing = Drawing::new();
    drawing.add_block(Block {
      name: "holes".to_owned(),
      entities: vec![Entity::new(EntityType::Circle(Circle {
        center: DxfPoint::new(40.0, 20.0, 0.0),
        radius: 5.0,
        normal: Vector::new(0.0, 0.0, -1.0),
        ..Default::default()
      }))],
      ..Default::default()
    });
    drawing.add_entity(Entity::new(EntityType::Insert(Insert {
      name: "holes".to_owned(),
      location: DxfPoint::new(100.0, 50.0, 0.0),
      x_scale_factor: 2.0,
      y_scale_factor: 2.0,
      extrusion_direction: Vector::new(0.0, 0.0, -1.0),
      ..Default::default()
    })));
    let (primitives, unsupported) = extract_primitives(&drawing);
    let bounds = primitives[0].bounds().unwrap();
    assert_eq!(unsupported, 0);
    assert_eq!(bounds.center(), Point::new(-20.0, 90.0));
    assert!((bounds.width() - 20.0).abs() < 0.001);
  }
}
