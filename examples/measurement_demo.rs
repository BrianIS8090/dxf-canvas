use dxf::{
  Drawing, LwPolylineVertex, Point,
  entities::{Circle, Entity, EntityType, LwPolyline},
  enums::{AcadVersion, Units},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut drawing = Drawing::new();
  drawing.header.version = AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;
  let bulge = (std::f64::consts::PI / 8.0).tan();
  let mut outline = LwPolyline {
    vertices: [
      (10.0, 0.0, 0.0),
      (150.0, 0.0, bulge),
      (160.0, 10.0, 0.0),
      (160.0, 90.0, bulge),
      (150.0, 100.0, 0.0),
      (10.0, 100.0, bulge),
      (0.0, 90.0, 0.0),
      (0.0, 10.0, bulge),
    ]
    .into_iter()
    .map(|(x, y, bulge)| LwPolylineVertex {
      x,
      y,
      bulge,
      ..Default::default()
    })
    .collect(),
    ..Default::default()
  };
  outline.set_is_closed(true);
  drawing.add_entity(Entity::new(EntityType::LwPolyline(outline)));
  for (x, radius) in [(40.0, 6.0), (110.0, 4.0)] {
    drawing.add_entity(Entity::new(EntityType::Circle(Circle {
      center: Point::new(x, 50.0, 0.0),
      radius,
      ..Default::default()
    })));
  }
  let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/measurement_demo.dxf");
  drawing.save_file(path)?;
  Ok(())
}
