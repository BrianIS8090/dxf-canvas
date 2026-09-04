use crate::geometry::{DrawingItem, MeasureCurve, Point, Primitive, RoundCurve, ViewTransform};
use eframe::egui::Pos2;

const SNAP_RADIUS: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
  Endpoint,
  Midpoint,
  Center,
  Quadrant,
  Perpendicular,
  Contour,
}

impl SnapKind {
  pub fn label(self) -> &'static str {
    match self {
      Self::Endpoint => "Конец",
      Self::Midpoint => "Середина",
      Self::Center => "Центр",
      Self::Quadrant => "Квадрант",
      Self::Perpendicular => "Перпендикуляр",
      Self::Contour => "На контуре",
    }
  }
}

#[derive(Clone, Copy, Debug)]
pub struct Snap {
  pub item: usize,
  pub point: Point,
  pub kind: SnapKind,
  pub approximate: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct RoundPick {
  pub item: usize,
  pub curve: RoundCurve,
}

pub fn pick_round(
  items: &[DrawingItem],
  transform: ViewTransform,
  pointer: Pos2,
  diameter: bool,
) -> Option<RoundPick> {
  let mut best = None;
  let mut best_distance = SNAP_RADIUS;
  for (index, item) in items.iter().enumerate().rev() {
    let local = item.local_point(transform.screen_to_world(pointer));
    for primitive_index in nearby_primitives(item, transform, local) {
      let primitive = &item.primitives[primitive_index];
      if !item.appearance.primitive_visible(primitive_index) {
        continue;
      }
      if let Primitive::Path { curves, .. } = primitive {
        for curve in curves {
          if let MeasureCurve::Round(curve) = curve {
            if diameter && !curve.is_full() {
              continue;
            }
            let nearest = curve.nearest(local);
            let distance = transform
              .world_to_screen(item.world_point(nearest))
              .distance(pointer);
            if distance < best_distance {
              best_distance = distance;
              best = Some(RoundPick {
                item: index,
                curve: *curve,
              });
            }
          }
        }
      }
    }
  }
  best
}

pub fn snap_point(
  items: &[DrawingItem],
  transform: ViewTransform,
  pointer: Pos2,
  owner: Option<usize>,
) -> Option<Snap> {
  snap_point_from(items, transform, pointer, owner, None)
}

fn snap_point_from(
  items: &[DrawingItem],
  transform: ViewTransform,
  pointer: Pos2,
  owner: Option<usize>,
  reference: Option<Point>,
) -> Option<Snap> {
  let mut best = None;
  let mut best_score = f32::INFINITY;
  for (index, item) in items.iter().enumerate().rev() {
    if owner.is_some_and(|owner| owner != index) {
      continue;
    }
    let local = item.local_point(transform.screen_to_world(pointer));
    let mut offer = |point: Point, kind: SnapKind, approximate: bool| {
      let distance = transform
        .world_to_screen(item.world_point(point))
        .distance(pointer);
      let score = distance
        + if kind == SnapKind::Contour {
          SNAP_RADIUS * 2.0
        } else {
          0.0
        };
      if distance <= SNAP_RADIUS && score < best_score {
        best_score = score;
        best = Some(Snap {
          item: index,
          point,
          kind,
          approximate,
        });
      }
    };
    for primitive_index in nearby_primitives(item, transform, local) {
      let primitive = &item.primitives[primitive_index];
      if !item.appearance.primitive_visible(primitive_index) {
        continue;
      }
      match primitive {
        Primitive::Point(point) => offer(*point, SnapKind::Endpoint, false),
        Primitive::Path { curves, .. } => {
          for curve in curves {
            match curve {
              MeasureCurve::Line { start, end } => {
                if let Some(reference) = reference
                  && let Some(foot) = perpendicular_on_segment(reference, *start, *end)
                  && distance(reference, foot) > 1.0e-9
                {
                  offer(foot, SnapKind::Perpendicular, false);
                }
                offer(*start, SnapKind::Endpoint, false);
                offer(*end, SnapKind::Endpoint, false);
                offer(
                  Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5),
                  SnapKind::Midpoint,
                  false,
                );
                offer(
                  nearest_on_segment(local, *start, *end),
                  SnapKind::Contour,
                  false,
                );
              }
              MeasureCurve::Round(curve) => {
                offer(curve.center, SnapKind::Center, curve.approximate);
                if !curve.is_full() {
                  offer(
                    curve.point_at(curve.start),
                    SnapKind::Endpoint,
                    curve.approximate,
                  );
                  offer(
                    curve.point_at(curve.start + curve.sweep),
                    SnapKind::Endpoint,
                    curve.approximate,
                  );
                  offer(
                    curve.point_at(curve.start + curve.sweep * 0.5),
                    SnapKind::Midpoint,
                    curve.approximate,
                  );
                }
                for quarter in 0..4 {
                  let angle = quarter as f64 * std::f64::consts::FRAC_PI_2;
                  if curve.contains_angle(angle) {
                    offer(curve.point_at(angle), SnapKind::Quadrant, curve.approximate);
                  }
                }
                offer(curve.nearest(local), SnapKind::Contour, curve.approximate);
              }
              MeasureCurve::Polyline { points, closed } => {
                if !closed {
                  if let Some(point) = points.first() {
                    offer(*point, SnapKind::Endpoint, false);
                  }
                  if let Some(point) = points.last() {
                    offer(*point, SnapKind::Endpoint, false);
                  }
                }
                for pair in points.windows(2) {
                  offer(
                    nearest_on_segment(local, pair[0], pair[1]),
                    SnapKind::Contour,
                    true,
                  );
                }
                if *closed && points.len() > 1 {
                  offer(
                    nearest_on_segment(local, points[points.len() - 1], points[0]),
                    SnapKind::Contour,
                    true,
                  );
                }
              }
            }
          }
        }
      }
    }
  }
  best
}

fn nearby_primitives(item: &DrawingItem, transform: ViewTransform, local: Point) -> Vec<usize> {
  if item.appearance.snap_index.matches(item.primitives.len()) {
    let radius =
      (SNAP_RADIUS as f64 + 0.1) / (transform.scale as f64 * item.scale).abs().max(1.0e-12);
    item
      .appearance
      .snap_index
      .query(crate::spatial::neighborhood(local, radius))
  } else {
    (0..item.primitives.len()).collect()
  }
}

fn perpendicular_on_segment(point: Point, start: Point, end: Point) -> Option<Point> {
  let dx = end.x - start.x;
  let dy = end.y - start.y;
  let denominator = dx * dx + dy * dy;
  if denominator < 1.0e-20 {
    return None;
  }
  let t = ((point.x - start.x) * dx + (point.y - start.y) * dy) / denominator;
  // Привязка только к самому отрезку, не к его воображаемому продолжению.
  (0.0..=1.0)
    .contains(&t)
    .then(|| Point::new(start.x + t * dx, start.y + t * dy))
}

fn nearest_on_segment(point: Point, start: Point, end: Point) -> Point {
  let dx = end.x - start.x;
  let dy = end.y - start.y;
  let denominator = dx * dx + dy * dy;
  if denominator < 1.0e-20 {
    return start;
  }
  let t = (((point.x - start.x) * dx + (point.y - start.y) * dy) / denominator).clamp(0.0, 1.0);
  Point::new(start.x + t * dx, start.y + t * dy)
}

#[derive(Clone, Debug)]
pub struct Dimension {
  pub item: usize,
  pub kind: DimensionKind,
  pub label: Point,
  pub approximate: bool,
}

#[derive(Clone, Debug)]
pub enum DimensionKind {
  Linear {
    start: Point,
    end: Point,
  },
  Round {
    curve: RoundCurve,
    diameter: bool,
  },
  Angle {
    first: Point,
    vertex: Point,
    last: Point,
  },
  Region(std::sync::Arc<crate::region::RegionMeasurement>),
}

impl Dimension {
  pub fn linear(item: usize, start: Point, end: Point, label: Point) -> Self {
    Self {
      item,
      kind: DimensionKind::Linear { start, end },
      label,
      approximate: false,
    }
  }

  pub fn value(&self) -> f64 {
    match self.kind {
      DimensionKind::Linear { start, end } => distance(start, end),
      DimensionKind::Round { curve, diameter } => curve.radius * if diameter { 2.0 } else { 1.0 },
      DimensionKind::Angle {
        first,
        vertex,
        last,
      } => angle_radians(first, vertex, last).abs().to_degrees(),
      DimensionKind::Region(ref region) => region.area,
    }
  }

  pub fn round(pick: RoundPick, diameter: bool, label: Point) -> Self {
    Self {
      item: pick.item,
      kind: DimensionKind::Round {
        curve: pick.curve,
        diameter,
      },
      label,
      approximate: pick.curve.approximate,
    }
  }

  pub fn text(&self, item: &DrawingItem) -> String {
    if let DimensionKind::Region(region) = &self.kind {
      let f = item.units.factor();
      return format!(
        "{}S = {} {}²\nP = {} {} · отверстий: {}",
        if self.approximate { "≈ " } else { "" },
        number(region.area * f * f),
        item.units.label(),
        number(region.perimeter * f),
        item.units.label(),
        region.holes
      );
    }
    if matches!(self.kind, DimensionKind::Angle { .. }) {
      return format!(
        "{}{}°",
        if self.approximate { "≈ " } else { "" },
        number(self.value())
      );
    }
    let prefix = match self.kind {
      DimensionKind::Linear { .. } => "",
      DimensionKind::Round { diameter: true, .. } => "Ø ",
      DimensionKind::Round {
        diameter: false, ..
      } => "R ",
      DimensionKind::Angle { .. } | DimensionKind::Region(_) => unreachable!(),
    };
    let value = self.value() * item.units.factor();
    let digits = if value.abs() < 0.001 { 6 } else { 3 };
    let number = format!("{value:.digits$}");
    let number = number
      .trim_end_matches('0')
      .trim_end_matches('.')
      .replace('.', ",");
    format!(
      "{}{prefix}{number} {}",
      if self.approximate { "≈ " } else { "" },
      item.units.label()
    )
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
  #[default]
  Select,
  Linear,
  Diameter,
  Radius,
  Angle,
  Region,
}

#[derive(Clone, Debug)]
enum Draft {
  Start(Snap),
  AngleVertex { first: Snap, vertex: Snap },
  Place(Dimension),
}

#[derive(Default)]
pub struct MeasurementState {
  pub tool: Tool,
  pub completed: Vec<Dimension>,
  draft: Option<Draft>,
  pub notice: Option<String>,
}

impl MeasurementState {
  pub fn set_tool(&mut self, tool: Tool) {
    self.tool = tool;
    self.draft = None;
    self.notice = None;
  }

  pub fn start_snap(&self) -> Option<Snap> {
    match self.draft {
      Some(Draft::Start(snap)) => Some(snap),
      Some(Draft::AngleVertex { vertex, .. }) => Some(vertex),
      _ => None,
    }
  }

  pub fn hover_snap(
    &self,
    items: &[DrawingItem],
    transform: ViewTransform,
    pointer: Pos2,
  ) -> Option<Snap> {
    if !matches!(self.tool, Tool::Linear | Tool::Angle) {
      return None;
    }
    let owner = match self.draft {
      Some(Draft::Start(snap)) => Some(snap.item),
      Some(Draft::AngleVertex { first, .. }) => Some(first.item),
      Some(Draft::Place(_)) => return None,
      None => None,
    };
    if let Some(start) = self.start_snap()
      && self.tool == Tool::Linear
    {
      snap_point_from(items, transform, pointer, owner, Some(start.point))
    } else {
      snap_point(items, transform, pointer, owner)
    }
  }

  pub fn hover_round(
    &self,
    items: &[DrawingItem],
    transform: ViewTransform,
    pointer: Pos2,
  ) -> Option<RoundPick> {
    if self.draft.is_some() || !matches!(self.tool, Tool::Diameter | Tool::Radius) {
      return None;
    }
    pick_round(items, transform, pointer, self.tool == Tool::Diameter)
  }

  pub fn click(&mut self, items: &[DrawingItem], transform: ViewTransform, pointer: Pos2) {
    self.notice = None;
    if let Some(Draft::Place(mut dimension)) = self.draft.clone() {
      if let Some(item) = items.get(dimension.item) {
        dimension.label = item.local_point(transform.screen_to_world(pointer));
        self.completed.push(dimension);
      }
      self.draft = None;
      return;
    }
    if self.tool == Tool::Region {
      for (index, item) in items.iter().enumerate().rev() {
        let local = item.local_point(transform.screen_to_world(pointer));
        let b = item.bounds;
        if local.x < b.min.x || local.x > b.max.x || local.y < b.min.y || local.y > b.max.y {
          continue;
        }
        match crate::region::measure_region(item, local) {
          Ok(region) => {
            self.draft = Some(Draft::Place(Dimension {
              item: index,
              label: local,
              approximate: region.approximate,
              kind: DimensionKind::Region(std::sync::Arc::new(region)),
            }));
          }
          Err(error) => self.notice = Some(error),
        }
        return;
      }
      self.notice = Some("Щёлкните внутри замкнутой детали на видимом слое.".into());
      return;
    }
    if let Some(snap) = self.hover_snap(items, transform, pointer) {
      if let Some(Draft::AngleVertex { first, vertex }) = self.draft {
        if distance(vertex.point, snap.point) > 1e-9 {
          self.draft = Some(Draft::Place(angle_dimension(first, vertex, snap)));
        }
        return;
      }
      if let Some(Draft::Start(start)) = self.draft {
        if distance(start.point, snap.point) > 1.0e-9 {
          if self.tool == Tool::Angle {
            self.draft = Some(Draft::AngleVertex {
              first: start,
              vertex: snap,
            });
            return;
          }
          let mut dimension = Dimension::linear(start.item, start.point, snap.point, snap.point);
          dimension.approximate = start.approximate || snap.approximate;
          self.draft = Some(Draft::Place(dimension));
        }
      } else {
        self.draft = Some(Draft::Start(snap));
      }
    } else if let Some(pick) = self.hover_round(items, transform, pointer) {
      let local = items[pick.item].local_point(transform.screen_to_world(pointer));
      self.draft = Some(Draft::Place(Dimension::round(
        pick,
        self.tool == Tool::Diameter,
        local,
      )));
    }
  }

  pub fn preview(
    &self,
    items: &[DrawingItem],
    transform: ViewTransform,
    pointer: Pos2,
  ) -> Option<Dimension> {
    match &self.draft {
      Some(Draft::Place(dimension)) => {
        let mut result = dimension.clone();
        result.label = items
          .get(result.item)?
          .local_point(transform.screen_to_world(pointer));
        Some(result)
      }
      Some(Draft::Start(start)) => {
        if self.tool == Tool::Angle {
          return None;
        }
        let snap = self.hover_snap(items, transform, pointer)?;
        if distance(start.point, snap.point) < 1.0e-9 {
          return None;
        }
        let mut result = Dimension::linear(start.item, start.point, snap.point, snap.point);
        result.approximate = start.approximate || snap.approximate;
        Some(result)
      }
      Some(Draft::AngleVertex { first, vertex }) => {
        let snap = self.hover_snap(items, transform, pointer)?;
        (distance(vertex.point, snap.point) > 1e-9).then(|| angle_dimension(*first, *vertex, snap))
      }
      None => None,
    }
  }

  pub fn cancel(&mut self) {
    self.notice = None;
    if self.draft.take().is_none() {
      self.tool = Tool::Select;
    }
  }

  pub fn undo(&mut self) {
    if self.draft.take().is_none() {
      self.completed.pop();
    }
  }

  pub fn clear(&mut self) {
    self.notice = None;
    self.completed.clear();
    self.draft = None;
  }

  pub fn remove_item(&mut self, index: usize) {
    self.draft = None;
    self.completed.retain(|dimension| dimension.item != index);
    for dimension in &mut self.completed {
      if dimension.item > index {
        dimension.item -= 1;
      }
    }
  }

  pub fn hint(&self) -> &'static str {
    match (&self.draft, self.tool) {
      (Some(Draft::Place(_)), _) => "Расположите размер и щёлкните · Esc — отмена",
      (Some(Draft::Start(_)), Tool::Angle) => {
        "Угол: выберите вершину (вторую точку) · Esc — отмена"
      }
      (Some(Draft::AngleVertex { .. }), _) => {
        "Угол: выберите третью точку на втором луче · Esc — отмена"
      }
      (Some(Draft::Start(_)), _) => {
        "Вторая точка на детали · Доступен «Перпендикуляр» · Esc — отмена"
      }
      (_, Tool::Linear) => "Выберите первую точку с привязкой · Размер по исходному DXF",
      (_, Tool::Diameter) => "Щёлкните по контуру круглого отверстия · Затем расположите размер",
      (_, Tool::Radius) => "Щёлкните по круговой дуге скругления · Затем расположите размер",
      (_, Tool::Angle) => {
        "Угол: первая точка → вершина → третья точка → размещение · Меньший угол 0–180°"
      }
      (_, Tool::Region) => {
        "Щёлкните внутри детали · S — площадь без отверстий · P — длина всех границ, включая отверстия · Только видимые слои"
      }
      (_, Tool::Select) => {
        "ЛКМ — двигать деталь · Маркеры / Ctrl+колесо — размер детали · Колесо — масштаб холста"
      }
    }
  }
}

fn number(value: f64) -> String {
  let digits = if value.abs() < 0.001 { 6 } else { 3 };
  format!("{value:.digits$}")
    .trim_end_matches('0')
    .trim_end_matches('.')
    .replace('.', ",")
}

pub fn angle_radians(first: Point, vertex: Point, last: Point) -> f64 {
  let a = Point::new(first.x - vertex.x, first.y - vertex.y);
  let b = Point::new(last.x - vertex.x, last.y - vertex.y);
  (a.x * b.y - a.y * b.x).atan2(a.x * b.x + a.y * b.y)
}

fn angle_dimension(first: Snap, vertex: Snap, last: Snap) -> Dimension {
  Dimension {
    item: first.item,
    label: last.point,
    kind: DimensionKind::Angle {
      first: first.point,
      vertex: vertex.point,
      last: last.point,
    },
    approximate: first.approximate || vertex.approximate || last.approximate,
  }
}

pub fn distance(a: Point, b: Point) -> f64 {
  (a.x - b.x).hypot(a.y - b.y)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::geometry::{DrawingItem, Point, ViewTransform};
  use eframe::egui::{Pos2, Vec2};

  fn imported(drawing: dxf::Drawing) -> DrawingItem {
    let stamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_nanos();
    let path = std::env::temp_dir().join(format!("dxf_measurement_{stamp}.dxf"));
    drawing.save_file(&path).unwrap();
    let result = crate::dxf_import::load_dxf(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    result
  }

  #[test]
  fn hidden_layers_do_not_offer_snaps_diameters_or_geometry_warnings() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Circle(dxf::entities::Circle {
        radius: 10.0,
        ..Default::default()
      }),
    ));
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(dxf::Point::origin(), dxf::Point::new(100.0, 0.0, 0.0)),
    )));
    let mut item = imported(drawing);
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let cursor = transform.world_to_screen(Point::new(10.0, 0.0));
    assert!(pick_round(std::slice::from_ref(&item), transform, cursor, true).is_some());
    for layer in &mut item.appearance.layers {
      layer.visible = false;
    }
    assert!(pick_round(std::slice::from_ref(&item), transform, cursor, true).is_none());
    assert!(snap_point(std::slice::from_ref(&item), transform, cursor, None).is_none());
    let report = crate::diagnostics::analyze(&item);
    assert!(
      report
        .findings
        .iter()
        .all(|finding| finding.kind == crate::diagnostics::IssueKind::UnknownUnits)
    );
  }

  #[test]
  fn indexed_snapping_matches_full_search_after_move_scale_and_layer_changes() {
    use dxf::entities::{Arc as DxfArc, Circle, Entity, EntityType, Line};
    let mut drawing = dxf::Drawing::new();
    for i in 0..80 {
      let x = (i % 10) as f64 * 20.0;
      let y = (i / 10) as f64 * 20.0;
      drawing.add_entity(Entity::new(EntityType::Circle(Circle {
        center: dxf::Point::new(x, y, 0.0),
        radius: 5.0,
        ..Default::default()
      })));
      let mut line = Entity::new(EntityType::Line(Line::new(
        dxf::Point::new(x - 8.0, y + 7.0, 0.0),
        dxf::Point::new(x + 8.0, y + 7.0, 0.0),
      )));
      line.common.layer = "Lines".into();
      drawing.add_entity(line);
    }
    drawing.add_entity(Entity::new(EntityType::Arc(DxfArc {
      center: dxf::Point::new(400.0, 300.0, 0.0),
      radius: 100.0,
      start_angle: 0.0,
      end_angle: 20.0,
      ..Default::default()
    })));
    let mut indexed = imported(drawing);
    assert!(
      indexed
        .appearance
        .snap_index
        .matches(indexed.primitives.len())
    );
    for layer in &mut indexed.appearance.layers {
      if layer.name == "Lines" {
        layer.visible = false;
      }
    }
    for item_scale in [0.1, 1.0, 4.0] {
      indexed.scale = item_scale;
      indexed.offset = Point::new(500.0, -250.0);
      let mut exhaustive = indexed.clone();
      exhaustive.appearance.snap_index = Default::default();
      for view_scale in [0.1, 1.0, 10.0] {
        let view = ViewTransform {
          scale: view_scale,
          origin: Pos2::new(100.0, 300.0),
        };
        for local in (0..150)
          .map(|i| Point::new((i % 15) as f64 * 14.0 - 8.0, (i / 15) as f64 * 16.0))
          .chain([Point::new(400.0, 300.0), Point::new(500.0, 300.0)])
        {
          let cursor = view.world_to_screen(indexed.world_point(local));
          let snap = |item: &DrawingItem| {
            snap_point_from(
              std::slice::from_ref(item),
              view,
              cursor,
              None,
              Some(Point::default()),
            )
            .map(|snap| (snap.kind, snap.point, snap.approximate))
          };
          assert_eq!(snap(&indexed), snap(&exhaustive));
          for diameter in [true, false] {
            let round = |item: &DrawingItem| {
              pick_round(std::slice::from_ref(item), view, cursor, diameter)
                .map(|pick| (pick.curve.center, pick.curve.radius))
            };
            assert_eq!(round(&indexed), round(&exhaustive));
          }
        }
      }
    }
  }

  #[test]
  fn aligned_dimension_reports_distance_between_source_points() {
    let dimension = Dimension::linear(
      0,
      Point::new(10.0, 20.0),
      Point::new(40.0, 60.0),
      Point::new(70.0, 80.0),
    );
    assert_eq!(dimension.value(), 50.0);
  }

  #[test]
  fn second_point_snaps_to_the_perpendicular_between_parallel_lines() {
    let mut drawing = dxf::Drawing::new();
    for y in [0.0, 40.0] {
      drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
        dxf::entities::Line::new(dxf::Point::new(0.0, y, 0.0), dxf::Point::new(100.0, y, 0.0)),
      )));
    }
    let items = vec![imported(drawing)];
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let mut state = MeasurementState::default();
    state.set_tool(Tool::Linear);
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::new(23.0, 0.0)),
    );
    let cursor = transform.world_to_screen(Point::new(24.5, 40.8));
    let snap = state.hover_snap(&items, transform, cursor).unwrap();
    assert_eq!(snap.kind.label(), "Перпендикуляр");
    assert_eq!(snap.point, Point::new(23.0, 40.0));
    state.click(&items, transform, cursor);
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::new(10.0, 20.0)),
    );
    assert_eq!(state.completed[0].value(), 40.0);
  }

  #[test]
  fn perpendicular_to_inclined_polyline_keeps_source_distance_after_move_and_scale() {
    let mut drawing = dxf::Drawing::new();
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    for vertices in [[(6.0, -8.0), (86.0, 52.0)], [(0.0, 0.0), (80.0, 60.0)]] {
      drawing.add_entity(dxf::entities::Entity::new(
        dxf::entities::EntityType::LwPolyline(dxf::entities::LwPolyline {
          vertices: vertices
            .into_iter()
            .map(|(x, y)| dxf::LwPolylineVertex {
              x,
              y,
              ..Default::default()
            })
            .collect(),
          ..Default::default()
        }),
      ));
    }
    let mut item = imported(drawing);
    item.scale = 2.4;
    item.offset = Point::new(500.0, -300.0);
    let items = vec![item];
    let transform = ViewTransform {
      scale: 3.2,
      origin: Pos2::new(100.0, 300.0),
    };
    let screen = |point| transform.world_to_screen(items[0].world_point(point));
    let mut state = MeasurementState::default();
    state.set_tool(Tool::Linear);
    state.click(&items, transform, screen(Point::new(30.0, 10.0)));
    let cursor = screen(Point::new(24.0, 18.0)) + Vec2::new(3.0, 2.0);
    let snap = state.hover_snap(&items, transform, cursor).unwrap();
    assert_eq!(snap.kind, SnapKind::Perpendicular);
    assert!(distance(snap.point, Point::new(24.0, 18.0)) < 0.0001);
    state.click(&items, transform, cursor);
    state.click(&items, transform, screen(Point::new(20.0, 20.0)));
    assert!((state.completed[0].value() - 10.0).abs() < 0.0001);
  }

  #[test]
  fn perpendicular_does_not_snap_to_an_extension_of_the_segment() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(
        dxf::Point::new(-20.0, 0.0, 0.0),
        dxf::Point::new(-20.0, 10.0, 0.0),
      ),
    )));
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(
        dxf::Point::new(0.0, 40.0, 0.0),
        dxf::Point::new(100.0, 40.0, 0.0),
      ),
    )));
    let items = vec![imported(drawing)];
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let mut state = MeasurementState::default();
    state.set_tool(Tool::Linear);
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::new(-20.0, 0.0)),
    );
    assert!(
      state
        .hover_snap(
          &items,
          transform,
          transform.world_to_screen(Point::new(-20.0, 40.0))
        )
        .is_none()
    );
    let endpoint = state
      .hover_snap(
        &items,
        transform,
        transform.world_to_screen(Point::new(0.0, 40.0)),
      )
      .unwrap();
    assert_eq!(endpoint.kind, SnapKind::Endpoint);
  }

  #[test]
  fn perpendicular_only_activates_for_second_point_within_screen_snap_radius() {
    let mut drawing = dxf::Drawing::new();
    for y in [0.0, 40.0] {
      drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
        dxf::entities::Line::new(dxf::Point::new(0.0, y, 0.0), dxf::Point::new(100.0, y, 0.0)),
      )));
    }
    let items = vec![imported(drawing)];
    for scale in [1.0, 4.0, 20.0] {
      let transform = ViewTransform {
        scale,
        origin: Pos2::ZERO,
      };
      let mut state = MeasurementState::default();
      state.set_tool(Tool::Linear);
      let first = transform.world_to_screen(Point::new(23.0, 0.0));
      assert_eq!(
        state.hover_snap(&items, transform, first).unwrap().kind,
        SnapKind::Contour
      );
      state.click(&items, transform, first);
      let foot = transform.world_to_screen(Point::new(23.0, 40.0));
      assert_eq!(
        state
          .hover_snap(&items, transform, foot + Vec2::new(9.0, 0.0))
          .unwrap()
          .kind,
        SnapKind::Perpendicular
      );
      assert_eq!(
        state
          .hover_snap(&items, transform, foot + Vec2::new(11.0, 0.0))
          .unwrap()
          .kind,
        SnapKind::Contour
      );
    }
  }

  #[test]
  fn snapping_uses_source_endpoint_after_moving_and_scaling_detail() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(
        dxf::Point::new(10.0, 20.0, 0.0),
        dxf::Point::new(40.0, 60.0, 0.0),
      ),
    )));
    let mut item = imported(drawing);
    item.scale = 3.0;
    item.offset = Point::new(500.0, -200.0);
    let transform = ViewTransform {
      scale: 1.7,
      origin: Pos2::ZERO,
    };
    let cursor =
      transform.world_to_screen(item.world_point(Point::new(10.0, 20.0))) + Vec2::new(3.0, 2.0);
    let snap = snap_point(&[item], transform, cursor, None).unwrap();
    assert_eq!(snap.point, Point::new(10.0, 20.0));
    assert_eq!(snap.kind, SnapKind::Endpoint);
  }

  #[test]
  fn diameter_uses_exact_dxf_circle_with_reversed_normal() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Circle(dxf::entities::Circle {
        center: dxf::Point::new(40.0, 20.0, 0.0),
        radius: 2.5,
        normal: dxf::Vector::new(0.0, 0.0, -1.0),
        ..Default::default()
      }),
    ));
    let mut item = imported(drawing);
    item.scale = 3.0;
    let transform = ViewTransform {
      scale: 2.0,
      origin: Pos2::ZERO,
    };
    let cursor = transform.world_to_screen(item.world_point(Point::new(-42.5, 20.0)));
    let pick = pick_round(&[item], transform, cursor, true).unwrap();
    assert!(!pick.curve.approximate);
    assert_eq!(Dimension::round(pick, true, Point::default()).value(), 5.0);
  }

  #[test]
  fn empty_spline_does_not_change_the_previous_exact_circle() {
    let mut drawing = dxf::Drawing::new();
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Circle(dxf::entities::Circle {
        radius: 6.0,
        ..Default::default()
      }),
    ));
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Spline(Default::default()),
    ));
    let item = imported(drawing);
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let pick = pick_round(
      &[item],
      transform,
      transform.world_to_screen(Point::new(6.0, 0.0)),
      true,
    )
    .unwrap();
    assert!(!pick.curve.approximate);
    assert_eq!(pick.curve.radius, 6.0);
  }

  #[test]
  fn radius_uses_exact_dxf_arc() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Arc(
      dxf::entities::Arc {
        center: dxf::Point::new(10.0, 20.0, 0.0),
        radius: 12.0,
        start_angle: 0.0,
        end_angle: 90.0,
        ..Default::default()
      },
    )));
    let item = imported(drawing);
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let cursor = transform.world_to_screen(Point::new(22.0, 20.0));
    let pick = pick_round(&[item], transform, cursor, false).unwrap();
    assert!(!pick.curve.is_full());
    assert_eq!(
      Dimension::round(pick, false, Point::default()).value(),
      12.0
    );
  }

  #[test]
  fn radius_recognizes_polyline_bulge_as_exact_arc() {
    let mut drawing = dxf::Drawing::new();
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::LwPolyline(dxf::entities::LwPolyline {
        vertices: vec![
          dxf::LwPolylineVertex {
            x: 0.0,
            y: 0.0,
            bulge: 1.0,
            ..Default::default()
          },
          dxf::LwPolylineVertex {
            x: 10.0,
            y: 0.0,
            ..Default::default()
          },
        ],
        ..Default::default()
      }),
    ));
    let item = imported(drawing);
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let cursor = transform.world_to_screen(Point::new(5.0, -5.0));
    let pick = pick_round(&[item], transform, cursor, false).unwrap();
    assert_eq!(Dimension::round(pick, false, Point::default()).value(), 5.0);
  }

  #[test]
  fn linear_tool_snaps_to_circle_center() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Circle(dxf::entities::Circle {
        center: dxf::Point::new(10.0, 20.0, 0.0),
        radius: 20.0,
        ..Default::default()
      }),
    ));
    let item = imported(drawing);
    let transform = ViewTransform {
      scale: 2.0,
      origin: Pos2::ZERO,
    };
    let snap = snap_point(
      &[item],
      transform,
      transform.world_to_screen(Point::new(10.0, 20.0)),
      None,
    )
    .unwrap();
    assert_eq!(snap.point, Point::new(10.0, 20.0));
    assert_eq!(snap.kind, SnapKind::Center);
  }

  #[test]
  fn three_clicks_place_an_aligned_dimension_that_survives_visual_scaling() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(
        dxf::Point::new(10.0, 20.0, 0.0),
        dxf::Point::new(40.0, 60.0, 0.0),
      ),
    )));
    let mut items = vec![imported(drawing)];
    let transform = ViewTransform {
      scale: 2.0,
      origin: Pos2::ZERO,
    };
    let mut state = MeasurementState::default();
    state.set_tool(Tool::Linear);
    for point in [
      Point::new(10.0, 20.0),
      Point::new(40.0, 60.0),
      Point::new(70.0, 80.0),
    ] {
      state.click(&items, transform, transform.world_to_screen(point));
    }
    items[0].scale = 4.0;
    items[0].offset = Point::new(200.0, 100.0);
    assert_eq!(state.completed.len(), 1);
    assert_eq!(state.completed[0].value(), 50.0);
    assert_eq!(state.completed[0].label, Point::new(70.0, 80.0));
  }

  #[test]
  fn dimension_converts_explicit_dxf_inches_to_millimeters() {
    let mut drawing = dxf::Drawing::new();
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    drawing.header.default_drawing_units = dxf::enums::Units::Inches;
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(
        dxf::Point::new(0.0, 0.0, 0.0),
        dxf::Point::new(2.0, 0.0, 0.0),
      ),
    )));
    let item = imported(drawing);
    let dimension = Dimension::linear(
      0,
      Point::default(),
      Point::new(2.0, 0.0),
      Point::new(0.0, 1.0),
    );
    assert_eq!(dimension.text(&item), "50,8 мм");
  }

  fn rational_spline_drawing(x_scale: f64) -> dxf::Drawing {
    let mut drawing = dxf::Drawing::new();
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Spline(dxf::entities::Spline {
        degree_of_curve: 2,
        control_points: [
          (10.0, 0.0),
          (10.0, 10.0),
          (0.0, 10.0),
          (-10.0, 10.0),
          (-10.0, 0.0),
          (-10.0, -10.0),
          (0.0, -10.0),
          (10.0, -10.0),
          (10.0, 0.0),
        ]
        .into_iter()
        .map(|(x, y)| dxf::Point::new(x * x_scale, y, 0.0))
        .collect(),
        weight_values: vec![
          1.0,
          std::f64::consts::FRAC_1_SQRT_2,
          1.0,
          std::f64::consts::FRAC_1_SQRT_2,
          1.0,
          std::f64::consts::FRAC_1_SQRT_2,
          1.0,
          std::f64::consts::FRAC_1_SQRT_2,
          1.0,
        ],
        knot_values: vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 4.0],
        ..Default::default()
      }),
    ));
    drawing
  }

  #[test]
  fn circular_spline_hole_can_be_measured_with_approximation_mark() {
    let item = imported(rational_spline_drawing(1.0));
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let pick = pick_round(
      &[item],
      transform,
      transform.world_to_screen(Point::new(10.0, 0.0)),
      true,
    )
    .unwrap();
    assert!(pick.curve.approximate);
    assert!((Dimension::round(pick, true, Point::default()).value() - 20.0).abs() < 0.000001);
  }

  #[test]
  fn slightly_oval_spline_does_not_receive_a_fabricated_diameter_or_radius() {
    let item = imported(rational_spline_drawing(1.0213363278));
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let cursor = transform.world_to_screen(Point::new(10.213363278, 0.0));
    assert!(pick_round(std::slice::from_ref(&item), transform, cursor, true).is_none());
    assert!(pick_round(&[item], transform, cursor, false).is_none());
  }

  #[test]
  fn stretched_circle_is_not_reported_as_a_diameter() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_block(dxf::Block {
      name: "hole".into(),
      entities: vec![dxf::entities::Entity::new(
        dxf::entities::EntityType::Circle(dxf::entities::Circle {
          radius: 10.0,
          ..Default::default()
        }),
      )],
      ..Default::default()
    });
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Insert(dxf::entities::Insert {
        name: "hole".into(),
        x_scale_factor: 2.0,
        ..Default::default()
      }),
    ));
    let item = imported(drawing);
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    assert!(
      pick_round(
        &[item],
        transform,
        transform.world_to_screen(Point::new(20.0, 0.0)),
        true
      )
      .is_none()
    );
  }

  #[test]
  fn two_clicks_place_diameter_and_undo_removes_only_the_measurement() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(
      dxf::entities::EntityType::Circle(dxf::entities::Circle {
        radius: 6.0,
        ..Default::default()
      }),
    ));
    let items = vec![imported(drawing)];
    let transform = ViewTransform {
      scale: 3.0,
      origin: Pos2::ZERO,
    };
    let mut state = MeasurementState::default();
    state.set_tool(Tool::Diameter);
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::new(6.0, 0.0)),
    );
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::new(20.0, 20.0)),
    );
    assert_eq!(state.completed[0].value(), 12.0);
    state.undo();
    assert!(state.completed.is_empty());
    assert_eq!(items[0].primitives.len(), 1);
  }

  #[test]
  fn line_measurement_does_not_mix_independently_placed_files() {
    let mut drawing = dxf::Drawing::new();
    drawing.add_entity(dxf::entities::Entity::new(dxf::entities::EntityType::Line(
      dxf::entities::Line::new(dxf::Point::origin(), dxf::Point::new(100.0, 0.0, 0.0)),
    )));
    let item = imported(drawing);
    let mut second = item.clone();
    second.offset = Point::new(300.0, 0.0);
    let items = vec![item, second];
    let transform = ViewTransform {
      scale: 1.0,
      origin: Pos2::ZERO,
    };
    let mut state = MeasurementState::default();
    state.set_tool(Tool::Linear);
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::default()),
    );
    state.click(
      &items,
      transform,
      transform.world_to_screen(Point::new(300.0, 0.0)),
    );
    assert!(state.completed.is_empty());
    assert!(state.start_snap().is_some());
    state.cancel();
    assert!(state.start_snap().is_none());
  }
}
