use eframe::egui;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
  pub x: f64,
  pub y: f64,
}

impl Point {
  pub const fn new(x: f64, y: f64) -> Self {
    Self { x, y }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
  pub min: Point,
  pub max: Point,
}

impl Bounds {
  pub fn empty() -> Self {
    Self {
      min: Point::new(f64::INFINITY, f64::INFINITY),
      max: Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    }
  }

  pub fn from_points(points: impl IntoIterator<Item = Point>) -> Option<Self> {
    let mut bounds = Self::empty();
    for point in points {
      bounds.include(point);
    }
    bounds.is_valid().then_some(bounds)
  }

  pub fn include(&mut self, point: Point) {
    if !point.x.is_finite() || !point.y.is_finite() {
      return;
    }
    self.min.x = self.min.x.min(point.x);
    self.min.y = self.min.y.min(point.y);
    self.max.x = self.max.x.max(point.x);
    self.max.y = self.max.y.max(point.y);
  }

  pub fn include_bounds(&mut self, other: Self) {
    self.include(other.min);
    self.include(other.max);
  }

  pub fn is_valid(self) -> bool {
    self.min.x.is_finite()
      && self.min.y.is_finite()
      && self.max.x.is_finite()
      && self.max.y.is_finite()
  }

  pub fn width(self) -> f64 {
    (self.max.x - self.min.x).max(0.0)
  }

  pub fn height(self) -> f64 {
    (self.max.y - self.min.y).max(0.0)
  }

  pub fn center(self) -> Point {
    Point::new(
      (self.min.x + self.max.x) * 0.5,
      (self.min.y + self.max.y) * 0.5,
    )
  }

  pub fn translated(self, offset: Point) -> Self {
    Self {
      min: Point::new(self.min.x + offset.x, self.min.y + offset.y),
      max: Point::new(self.max.x + offset.x, self.max.y + offset.y),
    }
  }
}

#[derive(Clone, Debug)]
pub enum MeasureCurve {
  Line { start: Point, end: Point },
  Round(RoundCurve),
  Polyline { points: Vec<Point>, closed: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct RoundCurve {
  pub center: Point,
  pub radius: f64,
  pub start: f64,
  pub sweep: f64,
  pub approximate: bool,
}

impl RoundCurve {
  pub fn is_full(self) -> bool {
    self.sweep.abs() >= std::f64::consts::TAU - 1.0e-7
  }

  pub fn point_at(self, angle: f64) -> Point {
    Point::new(
      self.center.x + self.radius * angle.cos(),
      self.center.y + self.radius * angle.sin(),
    )
  }

  pub fn contains_angle(self, angle: f64) -> bool {
    self.is_full()
      || ((angle - self.start) * self.sweep.signum()).rem_euclid(std::f64::consts::TAU)
        <= self.sweep.abs() + 1.0e-9
  }

  pub fn nearest(self, point: Point) -> Point {
    let angle = (point.y - self.center.y).atan2(point.x - self.center.x);
    if self.contains_angle(angle) {
      self.point_at(angle)
    } else {
      let a = self.point_at(self.start);
      let b = self.point_at(self.start + self.sweep);
      if (a.x - point.x).hypot(a.y - point.y) < (b.x - point.x).hypot(b.y - point.y) {
        a
      } else {
        b
      }
    }
  }
}

#[derive(Clone, Debug)]
pub enum Primitive {
  Path {
    points: Vec<Point>,
    closed: bool,
    curves: Vec<MeasureCurve>,
  },
  Point(Point),
}

impl Primitive {
  pub fn bounds(&self) -> Option<Bounds> {
    match self {
      Self::Path { points, .. } => Bounds::from_points(points.iter().copied()),
      Self::Point(point) => Bounds::from_points([*point]),
    }
  }
}

#[derive(Clone, Debug)]
pub struct DrawingItem {
  pub appearance: crate::cad_scene::Appearance,
  pub units: LengthUnit,
  pub path: std::path::PathBuf,
  pub name: String,
  pub primitives: Vec<Primitive>,
  pub bounds: Bounds,
  pub offset: Point,
  pub scale: f64,
  pub unsupported_entities: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LengthUnit {
  millimeters: Option<f64>,
}

impl LengthUnit {
  pub fn from_dxf_code(code: i16) -> Self {
    let factors = [
      0.0,
      25.4,
      304.8,
      1_609_344.0,
      1.0,
      10.0,
      1000.0,
      1_000_000.0,
      0.0000254,
      0.0254,
      914.4,
      1.0e-7,
      1.0e-6,
      0.001,
      100.0,
      10_000.0,
      100_000.0,
      1.0e12,
      1.495978707e14,
      9.4607304725808e18,
      3.085677581491367e19,
      1_200_000.0 / 3937.0,
      100_000.0 / 3937.0,
      3_600_000.0 / 3937.0,
      6_336_000_000.0 / 3937.0,
    ];
    Self {
      millimeters: factors
        .get(code as usize)
        .copied()
        .filter(|factor| *factor > 0.0),
    }
  }

  pub fn factor(self) -> f64 {
    self.millimeters.unwrap_or(1.0)
  }

  pub fn is_known(self) -> bool {
    self.millimeters.is_some()
  }
  pub fn label(self) -> &'static str {
    if self.millimeters.is_some() {
      "мм"
    } else {
      "ед. DXF"
    }
  }
}

impl DrawingItem {
  pub fn scaled_bounds(&self) -> Bounds {
    let center = self.bounds.center();
    Bounds {
      min: Point::new(
        center.x + (self.bounds.min.x - center.x) * self.scale,
        center.y + (self.bounds.min.y - center.y) * self.scale,
      ),
      max: Point::new(
        center.x + (self.bounds.max.x - center.x) * self.scale,
        center.y + (self.bounds.max.y - center.y) * self.scale,
      ),
    }
  }

  pub fn placed_bounds(&self) -> Bounds {
    self.scaled_bounds().translated(self.offset)
  }

  pub fn world_point(&self, point: Point) -> Point {
    let center = self.bounds.center();
    Point::new(
      center.x + (point.x - center.x) * self.scale + self.offset.x,
      center.y + (point.y - center.y) * self.scale + self.offset.y,
    )
  }

  pub fn local_point(&self, world: Point) -> Point {
    let center = self.bounds.center();
    Point::new(
      center.x + (world.x - self.offset.x - center.x) / self.scale,
      center.y + (world.y - self.offset.y - center.y) / self.scale,
    )
  }

  pub fn set_scale_keeping_anchor(&mut self, scale: f64, local_anchor: Point, world_anchor: Point) {
    self.scale = scale;
    let moved_anchor = self.world_point(local_anchor);
    self.offset.x += world_anchor.x - moved_anchor.x;
    self.offset.y += world_anchor.y - moved_anchor.y;
  }
}

#[derive(Clone, Copy, Debug)]
pub struct ViewTransform {
  pub scale: f32,
  pub origin: egui::Pos2,
}

impl ViewTransform {
  pub fn world_to_screen(self, point: Point) -> egui::Pos2 {
    egui::pos2(
      self.origin.x + point.x as f32 * self.scale,
      self.origin.y - point.y as f32 * self.scale,
    )
  }

  pub fn screen_to_world(self, point: egui::Pos2) -> Point {
    Point::new(
      ((point.x - self.origin.x) / self.scale) as f64,
      ((self.origin.y - point.y) / self.scale) as f64,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bounds_ignore_non_finite_points() {
    let bounds = Bounds::from_points([
      Point::new(f64::NAN, 0.0),
      Point::new(-3.0, 4.0),
      Point::new(7.0, -2.0),
    ])
    .unwrap();

    assert_eq!(bounds.min, Point::new(-3.0, -2.0));
    assert_eq!(bounds.max, Point::new(7.0, 4.0));
  }

  #[test]
  fn item_can_be_moved_and_scaled_independently() {
    let mut item = DrawingItem {
      appearance: Default::default(),
      units: LengthUnit::default(),
      path: std::path::PathBuf::from("detail.dxf"),
      name: "detail".to_owned(),
      primitives: vec![],
      bounds: Bounds {
        min: Point::new(0.0, 0.0),
        max: Point::new(100.0, 50.0),
      },
      offset: Point::new(10.0, 20.0),
      scale: 2.0,
      unsupported_entities: 0,
    };

    assert_eq!(
      item.world_point(Point::new(0.0, 0.0)),
      Point::new(-40.0, -5.0)
    );
    item.offset = Point::new(30.0, 40.0);
    assert_eq!(
      item.world_point(Point::new(0.0, 0.0)),
      Point::new(-20.0, 15.0)
    );
  }
}
