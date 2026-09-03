use std::collections::HashMap;

use crate::geometry::{Bounds, DrawingItem, MeasureCurve, Point, Primitive};

pub const JOIN_TOLERANCE: f64 = 0.01;
pub const SHORT_LENGTH: f64 = 0.1;
const DUPLICATE_TOLERANCE: f64 = 0.001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
  OpenContour,
  UnjoinedContour,
  Duplicate,
  ShortSegment,
  Oval,
  UnknownUnits,
  IncompleteGeometry,
}

impl IssueKind {
  pub const ALL: [Self; 7] = [
    Self::OpenContour,
    Self::UnjoinedContour,
    Self::Duplicate,
    Self::ShortSegment,
    Self::Oval,
    Self::UnknownUnits,
    Self::IncompleteGeometry,
  ];

  pub fn label(self) -> &'static str {
    match self {
      Self::OpenContour => "Свободные концы",
      Self::UnjoinedContour => "Необъединённые контуры",
      Self::Duplicate => "Совпадающие участки",
      Self::ShortSegment => "Короткие участки",
      Self::Oval => "Овальность / искажение",
      Self::UnknownUnits => "Неизвестные единицы",
      Self::IncompleteGeometry => "Неполная геометрия",
    }
  }

  pub fn explanation(self) -> &'static str {
    match self {
      Self::OpenContour => {
        "Конец не соединён с другим концом в этом файле. Это может быть разрыв или намеренно открытая линия."
      }
      Self::UnjoinedContour => {
        "Концы совпадают, но замкнутая цепочка состоит из нескольких отдельных объектов DXF. Это не зазор: при необходимости объедините объекты в CAD. Одна замкнутая полилиния сюда не относится."
      }
      Self::Duplicate => {
        "Полностью совпадающие линии, дуги или контуры. Частичные наложения не проверяются."
      }
      Self::ShortSegment => {
        "Исходный участок короче 0,1 мм, включая нулевую длину. Деление кривой для показа на экране не считается отдельными участками."
      }
      Self::Oval => {
        "Эллипс или округлый контур с искажением формы, в том числе произвольный сплайн. Это приближённая проверка, а не доказательство ошибки: форма может быть задумана именно такой."
      }
      Self::UnknownUnits => {
        "DXF не задаёт распознанные единицы. Пороговые значения применены в единицах DXF, а не в миллиметрах. Рамка относится ко всему файлу."
      }
      Self::IncompleteGeometry => {
        "Есть неподдерживаемые сущности или некорректные координаты. Проверка охватывает только доступную двумерную геометрию; рамка относится ко всему файлу."
      }
    }
  }
}

#[derive(Clone, Debug)]
pub enum Marker {
  Point(Point),
  Curve { primitive: usize, curve: usize },
  Primitive(usize),
  Contour(Vec<usize>),
  File,
}

impl Marker {
  pub fn bounds(&self, item: &DrawingItem) -> Option<Bounds> {
    match self {
      Self::Point(point) => Bounds::from_points([*point]),
      Self::Primitive(index) => item.primitives.get(*index)?.bounds(),
      Self::File => item.bounds.is_valid().then_some(item.bounds),
      Self::Contour(indices) => {
        let mut bounds = Bounds::empty();
        for index in indices {
          bounds.include_bounds(item.primitives.get(*index)?.bounds()?);
        }
        bounds.is_valid().then_some(bounds)
      }
      Self::Curve { primitive, curve } => {
        let Primitive::Path { curves, .. } = item.primitives.get(*primitive)? else {
          return None;
        };
        match curves.get(*curve)? {
          MeasureCurve::Line { start, end } => Bounds::from_points([*start, *end]),
          MeasureCurve::Polyline { points, .. } => Bounds::from_points(points.iter().copied()),
          MeasureCurve::Round(round) => {
            let mut bounds = Bounds::from_points([
              round.point_at(round.start),
              round.point_at(round.start + round.sweep),
            ])?;
            for angle in [
              0.0,
              std::f64::consts::FRAC_PI_2,
              std::f64::consts::PI,
              3.0 * std::f64::consts::FRAC_PI_2,
            ] {
              if round.contains_angle(angle) {
                bounds.include(round.point_at(angle));
              }
            }
            Some(bounds)
          }
        }
      }
    }
  }

  pub fn focus_bounds(&self, item: &DrawingItem) -> Option<Bounds> {
    let bounds = self.bounds(item)?;
    let center = bounds.center();
    // Для точек и очень коротких участков оставляем обозримую окрестность вместо бесконечного увеличения.
    let minimum_span = if matches!(self, Self::Point(_)) {
      2.0
    } else {
      0.25
    } / item.units.factor();
    let half_width = bounds.width().max(minimum_span) * 0.5;
    let half_height = bounds.height().max(minimum_span) * 0.5;
    Bounds::from_points([
      item.world_point(Point::new(center.x - half_width, center.y - half_height)),
      item.world_point(Point::new(center.x + half_width, center.y + half_height)),
    ])
  }
}

#[derive(Clone, Debug)]
pub struct Finding {
  pub kind: IssueKind,
  pub marker: Marker,
  pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct DiagnosticReport {
  pub findings: Vec<Finding>,
}

impl DiagnosticReport {
  pub fn count(&self, kind: IssueKind) -> usize {
    self
      .findings
      .iter()
      .filter(|finding| finding.kind == kind)
      .count()
  }

  fn add(&mut self, kind: IssueKind, marker: Marker, detail: String) {
    self.findings.push(Finding {
      kind,
      marker,
      detail,
    });
  }
}

#[derive(Default)]
pub struct DiagnosticsState {
  pub enabled: bool,
  pub reports: Vec<DiagnosticReport>,
  pub selected: Option<DiagnosticSelection>,
  focus_request: Option<DiagnosticSelection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticSelection {
  pub item: usize,
  pub finding: usize,
}

impl DiagnosticsState {
  pub fn toggle(&mut self, items: &[DrawingItem]) {
    self.enabled = !self.enabled;
    self.refresh(items);
  }

  pub fn refresh(&mut self, items: &[DrawingItem]) {
    self.clear_selection();
    self.reports = if self.enabled {
      items.iter().map(analyze).collect()
    } else {
      Vec::new()
    };
  }

  pub fn clear(&mut self) {
    self.enabled = false;
    self.reports.clear();
    self.clear_selection();
  }

  pub fn select(&mut self, item: usize, finding: usize) -> bool {
    if !self.enabled
      || self
        .reports
        .get(item)
        .and_then(|report| report.findings.get(finding))
        .is_none()
    {
      return false;
    }
    let selection = DiagnosticSelection { item, finding };
    self.selected = Some(selection);
    self.focus_request = Some(selection);
    true
  }

  pub fn take_focus_request(&mut self) -> Option<DiagnosticSelection> {
    self.focus_request.take()
  }

  pub fn clear_selection(&mut self) {
    self.selected = None;
    self.focus_request = None;
  }
}

struct CurveRef<'a> {
  shape: &'a MeasureCurve,
  primitive: usize,
  curve: usize,
}

pub fn analyze(item: &DrawingItem) -> DiagnosticReport {
  let mut report = DiagnosticReport::default();
  let factor = item.units.factor();
  let join_tolerance = JOIN_TOLERANCE / factor;
  let duplicate_tolerance = DUPLICATE_TOLERANCE / factor;
  if !item.units.is_known() {
    report.add(
      IssueKind::UnknownUnits,
      Marker::File,
      "Единицы не заданы: размеры и допуски выражены в ед. DXF.".into(),
    );
  }
  if item.unsupported_entities > 0 {
    report.add(
      IssueKind::IncompleteGeometry,
      Marker::File,
      format!(
        "Не показано сущностей: {}. Проверка файла неполная.",
        item.unsupported_entities
      ),
    );
  }

  let mut curves = Vec::new();
  for (primitive_index, primitive) in item.primitives.iter().enumerate() {
    if let Primitive::Path {
      points,
      closed,
      curves: shapes,
    } = primitive
    {
      if points.iter().any(|point| !finite(*point)) {
        report.add(
          IssueKind::IncompleteGeometry,
          Marker::File,
          format!("Некорректные координаты элемента {}.", primitive_index + 1),
        );
        continue;
      }
      if (*closed || ends_meet(points, join_tolerance))
        && !matches!(shapes.as_slice(), [MeasureCurve::Round(_)])
      {
        let detail = if let Some((major, minor)) = ellipse_axes(points) {
          Some(format!(
            "Овальный контур ≈ {:.3} × {:.3} {}. Это предупреждение, не ошибка.",
            major * factor,
            minor * factor,
            item.units.label()
          ))
        } else if matches!(shapes.as_slice(), [MeasureCurve::Polyline { .. }])
          && distorted_round_contour(points, factor)
        {
          let bounds = Bounds::from_points(points.iter().copied()).unwrap();
          Some(format!(
            "Округлый контур с искажением формы. Габариты ≈ {:.3} × {:.3} {}. Постоянного диаметра нет; форма может быть намеренной.",
            bounds.width() * factor,
            bounds.height() * factor,
            item.units.label()
          ))
        } else {
          None
        };
        if let Some(detail) = detail {
          report.add(IssueKind::Oval, Marker::Primitive(primitive_index), detail);
        }
      }
      for (curve_index, curve) in shapes.iter().enumerate() {
        if !valid_curve(curve) {
          report.add(
            IssueKind::IncompleteGeometry,
            Marker::File,
            format!("Некорректный участок элемента {}.", primitive_index + 1),
          );
          continue;
        }
        let length = curve_length(curve) * factor;
        if length < SHORT_LENGTH {
          report.add(
            IssueKind::ShortSegment,
            Marker::Curve {
              primitive: primitive_index,
              curve: curve_index,
            },
            format!(
              "Короткий участок: {:.6} {} (< 0,1).",
              length,
              item.units.label()
            ),
          );
        }
        curves.push(CurveRef {
          shape: curve,
          primitive: primitive_index,
          curve: curve_index,
        });
      }
    }
  }

  let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
  let mut ends = Vec::new();
  let mut endpoint_curves = Vec::new();
  for (index, curve) in curves.iter().enumerate() {
    let key = cell(curve_anchor(curve.shape), duplicate_tolerance);
    let duplicate = neighbors(key).any(|key| {
      buckets.get(&key).is_some_and(|indices| {
        indices
          .iter()
          .any(|other| same_curve(curve.shape, curves[*other].shape, duplicate_tolerance))
      })
    });
    if duplicate {
      report.add(
        IssueKind::Duplicate,
        Marker::Curve {
          primitive: curve.primitive,
          curve: curve.curve,
        },
        format!(
          "Элемент {}, участок {}: найдено совпадение с другим участком этого файла.",
          curve.primitive + 1,
          curve.curve + 1
        ),
      );
    } else {
      buckets.entry(key).or_default().push(index);
      // Дубли не должны маскировать разрыв: связность проверяется по уникальным участкам.
      if curve_length(curve.shape) > 1.0e-12 / factor
        && let Some((start, end)) = curve_ends(curve.shape)
      {
        ends.extend([start, end]);
        endpoint_curves.push(index);
      }
    }
  }
  let groups = endpoint_groups(&ends, join_tolerance);
  let mut counts = vec![0; ends.len()];
  for group in &groups {
    counts[*group] += 1;
  }
  for (index, point) in ends
    .iter()
    .enumerate()
    .filter(|(index, _)| counts[*index] == 1)
  {
    debug_assert_eq!(groups[index], index);
    report.add(
      IssueKind::OpenContour,
      Marker::Point(*point),
      format!(
        "Свободный конец: X {:.3}, Y {:.3} {}. Не найден стык в пределах 0,01 {}.",
        point.x * factor,
        point.y * factor,
        item.units.label(),
        item.units.label()
      ),
    );
  }
  for primitives in separate_contours(&groups, &counts, &endpoint_curves, &curves) {
    let detail = format!(
      "Замкнутая цепочка из {} отдельных объектов DXF. Концы совпадают в пределах 0,01 {}, но объекты не объединены. Это не разрыв геометрии.",
      primitives.len(),
      item.units.label()
    );
    report.add(
      IssueKind::UnjoinedContour,
      Marker::Contour(primitives),
      detail,
    );
  }
  report
}

fn finite(point: Point) -> bool {
  point.x.is_finite() && point.y.is_finite()
}

fn dist(a: Point, b: Point) -> f64 {
  (a.x - b.x).hypot(a.y - b.y)
}

fn ends_meet(points: &[Point], tolerance: f64) -> bool {
  points.len() > 2 && dist(points[0], points[points.len() - 1]) <= tolerance
}

fn valid_curve(curve: &MeasureCurve) -> bool {
  match curve {
    MeasureCurve::Line { start, end } => finite(*start) && finite(*end),
    MeasureCurve::Round(curve) => {
      finite(curve.center)
        && curve.radius.is_finite()
        && curve.radius >= 0.0
        && curve.start.is_finite()
        && curve.sweep.is_finite()
    }
    MeasureCurve::Polyline { points, .. } => {
      points.len() >= 2 && points.iter().all(|point| finite(*point))
    }
  }
}

fn curve_length(curve: &MeasureCurve) -> f64 {
  match curve {
    MeasureCurve::Line { start, end } => dist(*start, *end),
    MeasureCurve::Round(curve) => curve.radius * curve.sweep.abs(),
    MeasureCurve::Polyline { points, closed } => {
      points
        .windows(2)
        .map(|pair| dist(pair[0], pair[1]))
        .sum::<f64>()
        + if *closed {
          dist(points[0], points[points.len() - 1])
        } else {
          0.0
        }
    }
  }
}

fn curve_ends(curve: &MeasureCurve) -> Option<(Point, Point)> {
  match curve {
    MeasureCurve::Line { start, end } => Some((*start, *end)),
    MeasureCurve::Round(curve) if !curve.is_full() => Some((
      curve.point_at(curve.start),
      curve.point_at(curve.start + curve.sweep),
    )),
    MeasureCurve::Polyline {
      points,
      closed: false,
    } => Some((points[0], points[points.len() - 1])),
    _ => None,
  }
}

fn curve_anchor(curve: &MeasureCurve) -> Point {
  match curve {
    MeasureCurve::Line { start, end } => {
      Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5)
    }
    MeasureCurve::Round(curve) => curve.center,
    MeasureCurve::Polyline { points, .. } => Bounds::from_points(points.iter().copied())
      .unwrap()
      .center(),
  }
}

fn same_curve(a: &MeasureCurve, b: &MeasureCurve, tolerance: f64) -> bool {
  let near = |a, b| dist(a, b) <= tolerance;
  match (a, b) {
    (MeasureCurve::Line { start: a, end: b }, MeasureCurve::Line { start: c, end: d }) => {
      (near(*a, *c) && near(*b, *d)) || (near(*a, *d) && near(*b, *c))
    }
    (MeasureCurve::Round(a), MeasureCurve::Round(b)) => {
      if !near(a.center, b.center)
        || (a.radius - b.radius).abs() > tolerance
        || a.is_full() != b.is_full()
      {
        return false;
      }
      if a.is_full() {
        return true;
      }
      let (a0, a1) = (a.point_at(a.start), a.point_at(a.start + a.sweep));
      let (b0, b1) = (b.point_at(b.start), b.point_at(b.start + b.sweep));
      near(
        a.point_at(a.start + a.sweep * 0.5),
        b.point_at(b.start + b.sweep * 0.5),
      ) && ((near(a0, b0) && near(a1, b1)) || (near(a0, b1) && near(a1, b0)))
    }
    (
      MeasureCurve::Polyline {
        points: a,
        closed: ac,
      },
      MeasureCurve::Polyline {
        points: b,
        closed: bc,
      },
    ) => {
      a.len() == b.len()
        && ac == bc
        && (a.iter().zip(b).all(|(a, b)| near(*a, *b))
          || a.iter().zip(b.iter().rev()).all(|(a, b)| near(*a, *b)))
    }
    _ => false,
  }
}

fn cell(point: Point, tolerance: f64) -> (i64, i64) {
  let quantize = |value: f64| (value / tolerance).floor().clamp(-9.0e18, 9.0e18) as i64;
  (quantize(point.x), quantize(point.y))
}

fn neighbors((x, y): (i64, i64)) -> impl Iterator<Item = (i64, i64)> {
  (-1..=1).flat_map(move |dx| (-1..=1).map(move |dy| (x + dx, y + dy)))
}

fn root(parents: &mut [usize], mut index: usize) -> usize {
  while parents[index] != index {
    parents[index] = parents[parents[index]];
    index = parents[index];
  }
  index
}

fn endpoint_groups(points: &[Point], tolerance: f64) -> Vec<usize> {
  let mut parents: Vec<_> = (0..points.len()).collect();
  let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
  for (index, point) in points.iter().enumerate() {
    let key = cell(*point, tolerance);
    for key in neighbors(key) {
      if let Some(others) = buckets.get(&key) {
        for other in others {
          if dist(*point, points[*other]) <= tolerance {
            let a = root(&mut parents, index);
            let b = root(&mut parents, *other);
            parents[a] = b;
          }
        }
      }
    }
    buckets.entry(key).or_default().push(index);
  }
  (0..points.len())
    .map(|index| root(&mut parents, index))
    .collect()
}

fn separate_contours(
  groups: &[usize],
  counts: &[usize],
  endpoint_curves: &[usize],
  curves: &[CurveRef<'_>],
) -> Vec<Vec<usize>> {
  let mut parents: Vec<_> = (0..endpoint_curves.len()).collect();
  let mut first_edge = HashMap::new();
  for (edge, ends) in groups.as_chunks::<2>().0.iter().enumerate() {
    for node in ends {
      if let Some(other) = first_edge.insert(*node, edge) {
        let a = root(&mut parents, edge);
        let b = root(&mut parents, other);
        parents[a] = b;
      }
    }
  }
  let mut components = vec![Vec::new(); endpoint_curves.len()];
  for edge in 0..endpoint_curves.len() {
    let component = root(&mut parents, edge);
    components[component].push(edge);
  }
  let mut result = Vec::new();
  for edges in components.into_iter().filter(|edges| !edges.is_empty()) {
    // Только замкнутые цепочки без свободных концов и разветвлений.
    if !edges.iter().all(|edge| {
      groups[edge * 2..edge * 2 + 2]
        .iter()
        .all(|node| counts[*node] == 2)
    }) {
      continue;
    }
    let mut primitives: Vec<_> = edges
      .iter()
      .map(|edge| curves[endpoint_curves[*edge]].primitive)
      .collect();
    primitives.sort_unstable();
    primitives.dedup();
    if primitives.len() > 1 {
      result.push(primitives);
    }
  }
  result.sort_unstable();
  result
}

fn distorted_round_contour(points: &[Point], factor: f64) -> bool {
  if points.len() < 16 {
    return false;
  }
  let Some(bounds) = Bounds::from_points(points.iter().copied()) else {
    return false;
  };
  let scale = bounds.width().max(bounds.height());
  if scale <= 1.0e-12 {
    return false;
  }
  let center = bounds.center();
  // Нормализация защищает расчёт площади от больших координат чертежа.
  let mut ring: Vec<_> = points
    .iter()
    .map(|point| Point::new((point.x - center.x) / scale, (point.y - center.y) / scale))
    .collect();
  ring.dedup_by(|a, b| dist(*a, *b) < 1.0e-10);
  if ring.len() > 1 && dist(ring[0], *ring.last().unwrap()) < 1.0e-8 {
    ring.pop();
  }
  if ring.len() < 16 {
    return false;
  }
  let mut twice_area = 0.0;
  let mut perimeter = 0.0;
  let mut min_radius = f64::INFINITY;
  let mut max_radius = 0.0_f64;
  for (i, point) in ring.iter().enumerate() {
    let previous = ring[(i + ring.len() - 1) % ring.len()];
    let next = ring[(i + 1) % ring.len()];
    let a = Point::new(point.x - previous.x, point.y - previous.y);
    let b = Point::new(next.x - point.x, next.y - point.y);
    let turn = (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y).abs();
    // Квадраты и другие угловатые контуры не называем искажёнными окружностями.
    if turn > std::f64::consts::PI / 9.0 {
      return false;
    }
    perimeter += dist(*point, next);
    twice_area += point.x * next.y - point.y * next.x;
    let radius = point.x.hypot(point.y);
    min_radius = min_radius.min(radius);
    max_radius = max_radius.max(radius);
  }
  let circularity = 2.0 * std::f64::consts::PI * twice_area.abs() / perimeter.powi(2);
  let threshold = (0.005 / factor / scale).max(max_radius * 0.002);
  circularity >= 0.94 && max_radius - min_radius > threshold
}

fn ellipse_axes(points: &[Point]) -> Option<(f64, f64)> {
  if points.len() < 16 {
    return None;
  }
  let bounds = Bounds::from_points(points.iter().copied())?;
  let origin = bounds.center();
  let scale = bounds.width().max(bounds.height()) * 0.5;
  if scale <= 1.0e-12 {
    return None;
  }
  // Подбираем уравнение эллипса в нормализованных координатах; поворот не влияет на результат.
  let mut system = [[0.0; 6]; 5];
  for point in points {
    let x = (point.x - origin.x) / scale;
    let y = (point.y - origin.y) / scale;
    let row = [x * x, x * y, y * y, x, y];
    for i in 0..5 {
      for j in 0..5 {
        system[i][j] += row[i] * row[j];
      }
      system[i][5] += row[i];
    }
  }
  for pivot in 0..5 {
    let best =
      (pivot..5).max_by(|a, b| system[*a][pivot].abs().total_cmp(&system[*b][pivot].abs()))?;
    system.swap(pivot, best);
    let divisor = system[pivot][pivot];
    if divisor.abs() < 1.0e-10 {
      return None;
    }
    for value in &mut system[pivot][pivot..] {
      *value /= divisor;
    }
    let pivot_row = system[pivot];
    for (i, row) in system.iter_mut().enumerate() {
      if i != pivot {
        let multiplier = row[pivot];
        for (value, pivot_value) in row[pivot..].iter_mut().zip(&pivot_row[pivot..]) {
          *value -= multiplier * pivot_value;
        }
      }
    }
  }
  let [a, b, c, d, e] = std::array::from_fn(|i| system[i][5]);
  let determinant = a * c - b * b * 0.25;
  if a <= 0.0 || c <= 0.0 || determinant <= 1.0e-10 {
    return None;
  }
  let cx = (-c * d + b * e * 0.5) / (2.0 * determinant);
  let cy = (b * d * 0.5 - a * e) / (2.0 * determinant);
  let k = 1.0 + a * cx * cx + b * cx * cy + c * cy * cy;
  let delta = (a - c).hypot(b);
  let minor_eigen = (a + c - delta) * 0.5;
  let major_eigen = (a + c + delta) * 0.5;
  if k <= 0.0 || minor_eigen <= 0.0 {
    return None;
  }
  let major = 2.0 * (k / minor_eigen).sqrt() * scale;
  let minor = 2.0 * (k / major_eigen).sqrt() * scale;
  if !major.is_finite() || major / minor < 1.001 {
    return None;
  }
  if points.iter().any(|point| {
    let x = (point.x - origin.x) / scale;
    let y = (point.y - origin.y) / scale;
    (a * x * x + b * x * y + c * y * y + d * x + e * y - 1.0).abs() > 0.002 * k
  }) {
    return None;
  }
  Some((major, minor))
}
