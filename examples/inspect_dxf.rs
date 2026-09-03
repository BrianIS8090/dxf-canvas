#[allow(dead_code)]
#[path = "../src/diagnostics.rs"]
mod diagnostics;
#[allow(dead_code)]
#[path = "../src/dxf_import.rs"]
mod dxf_import;
#[allow(dead_code)]
#[path = "../src/geometry.rs"]
mod geometry;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  for path in std::env::args_os().skip(1) {
    let item = dxf_import::load_dxf(std::path::Path::new(&path))?;
    let report = diagnostics::analyze(&item);
    println!("Проверка DXF: {} замечаний", report.findings.len());
    for finding in report.findings {
      println!("  {}: {}", finding.kind.label(), finding.detail);
    }
    println!(
      "{}\n  bounds: ({:.6}, {:.6}) — ({:.6}, {:.6}); size: {:.6} x {:.6}; primitives: {}; unsupported: {}",
      item.name,
      item.bounds.min.x,
      item.bounds.min.y,
      item.bounds.max.x,
      item.bounds.max.y,
      item.bounds.width(),
      item.bounds.height(),
      item.primitives.len(),
      item.unsupported_entities,
    );
    let rounds: Vec<_> = item
      .primitives
      .iter()
      .flat_map(|primitive| match primitive {
        geometry::Primitive::Path { curves, .. } => curves.as_slice(),
        geometry::Primitive::Point(_) => &[],
      })
      .filter_map(|curve| match curve {
        geometry::MeasureCurve::Round(curve) => Some(format!(
          "{}R {:.6}{}",
          if curve.approximate { "~" } else { "" },
          curve.radius,
          if curve.is_full() { " circle" } else { " arc" }
        )),
        _ => None,
      })
      .collect();
    println!("  round features ({}): {}", rounds.len(), rounds.join(", "));
    for primitive in &item.primitives {
      if let geometry::Primitive::Path { points, curves, .. } = primitive
        && matches!(curves.as_slice(), [geometry::MeasureCurve::Polyline { .. }])
        && let Some(bounds) = primitive.bounds()
      {
        let center = geometry::Point::new(
          (bounds.min.x + bounds.max.x) * 0.5,
          (bounds.min.y + bounds.max.y) * 0.5,
        );
        let radii: Vec<_> = points
          .iter()
          .map(|p| (p.x - center.x).hypot(p.y - center.y))
          .collect();
        println!(
          "  sampled curve: {} points, size {:.9} x {:.9}, radius {:.9}..{:.9}, end gap {:.9}",
          points.len(),
          bounds.width(),
          bounds.height(),
          radii.iter().copied().fold(f64::INFINITY, f64::min),
          radii.iter().copied().fold(0.0, f64::max),
          (points[0].x - points[points.len() - 1].x)
            .hypot(points[0].y - points[points.len() - 1].y)
        );
      }
    }
  }
  Ok(())
}
