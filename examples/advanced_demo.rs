use dxf::{
  Drawing, LwPolylineVertex, Point,
  entities::{Circle, Entity, EntityType, Line, LwPolyline},
  enums::{AcadVersion, Units},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut drawing = Drawing::new();
  drawing.header.version = AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;
  // Слева — корректная пластина 100 × 50 мм с отверстием Ø20 мм.
  // Справа — самопересечение и частичное наложение для проверки подсветки.
  for points in [
    vec![(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)],
    vec![(180.0, 0.0), (260.0, 80.0), (180.0, 80.0), (260.0, 0.0)],
  ] {
    let mut polyline = LwPolyline {
      vertices: points
        .into_iter()
        .map(|(x, y)| LwPolylineVertex {
          x,
          y,
          ..Default::default()
        })
        .collect(),
      ..Default::default()
    };
    polyline.set_is_closed(true);
    drawing.add_entity(Entity::new(EntityType::LwPolyline(polyline)));
  }
  drawing.add_entity(Entity::new(EntityType::Circle(Circle {
    center: Point::new(25.0, 25.0, 0.0),
    radius: 10.0,
    ..Default::default()
  })));
  for (a, b) in [
    ((160.0, 110.0), (260.0, 110.0)),
    ((210.0, 110.0), (290.0, 110.0)),
  ] {
    drawing.add_entity(Entity::new(EntityType::Line(Line::new(
      Point::new(a.0, a.1, 0.0),
      Point::new(b.0, b.1, 0.0),
    ))));
  }
  drawing.save_file(
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/advanced_demo.dxf"),
  )?;
  Ok(())
}
