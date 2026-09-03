use dxf::{
  Drawing, Point, Vector,
  entities::{Circle, Ellipse, Entity, EntityType, Line, Text},
  enums::{AcadVersion, Units},
};

fn line(drawing: &mut Drawing, start: (f64, f64), end: (f64, f64)) {
  drawing.add_entity(Entity::new(EntityType::Line(Line::new(
    Point::new(start.0, start.1, 0.0),
    Point::new(end.0, end.1, 0.0),
  ))));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
  let mut drawing = Drawing::new();
  drawing.header.version = AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;
  // Разрыв сверху, повтор нижней линии и короткий участок замкнутого треугольника.
  for (start, end) in [
    ((0.0, 0.0), (160.0, 0.0)),
    ((160.0, 0.0), (160.0, 100.0)),
    ((160.0, 100.0), (80.0, 100.0)),
    ((77.0, 100.0), (0.0, 100.0)),
    ((0.0, 100.0), (0.0, 0.0)),
    ((160.0, 0.0), (0.0, 0.0)),
    ((40.0, 50.0), (40.05, 50.0)),
    ((40.05, 50.0), (50.0, 60.0)),
    ((50.0, 60.0), (40.0, 50.0)),
  ] {
    line(&mut drawing, start, end);
  }
  drawing.add_entity(Entity::new(EntityType::Ellipse(Ellipse {
    center: Point::new(110.0, 50.0, 0.0),
    major_axis: Vector::new(12.0, 0.0, 0.0),
    minor_axis_ratio: 0.94,
    start_parameter: 0.0,
    end_parameter: std::f64::consts::TAU,
    ..Default::default()
  })));
  drawing.add_entity(Entity::new(EntityType::Circle(Circle {
    center: Point::new(20.0, 20.0, 0.0),
    radius: 5.0,
    ..Default::default()
  })));
  drawing.save_file(folder.join("diagnostics_demo.dxf"))?;

  let mut incomplete = Drawing::new();
  incomplete.header.version = AcadVersion::R2000;
  incomplete.header.default_drawing_units = Units::Unitless;
  incomplete.add_entity(Entity::new(EntityType::Circle(Circle {
    radius: 25.0,
    ..Default::default()
  })));
  // Текст пока не поддерживается импортом: должен появиться сигнал о неполной проверке.
  incomplete.add_entity(Entity::new(EntityType::Text(Text {
    value: "Контроль неподдерживаемой сущности".into(),
    text_height: 2.0,
    ..Default::default()
  })));
  incomplete.save_file(folder.join("diagnostics_units_demo.dxf"))?;
  Ok(())
}
