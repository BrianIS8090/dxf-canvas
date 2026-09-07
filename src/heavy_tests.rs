use crate::{
  diagnostics::{DiagnosticReport, Finding, IssueKind, Marker, analyze},
  geometry::{Point, ViewTransform},
};
use eframe::egui;

fn draw_report(item: &crate::geometry::DrawingItem, report: &DiagnosticReport) -> usize {
  let context = egui::Context::default();
  let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
  let scale = (900.0 / item.bounds.width()).min(600.0 / item.bounds.height()) as f32;
  let center = item.bounds.center();
  let view = ViewTransform {
    scale,
    origin: egui::pos2(
      500.0 - center.x as f32 * scale,
      350.0 + center.y as f32 * scale,
    ),
  };
  let mut output = context.run_ui(
    egui::RawInput {
      screen_rect: Some(rect),
      ..Default::default()
    },
    |ui| {
      crate::diagnostics_ui::paint_report(ui.painter(), item, report, view, None);
    },
  );
  let commands = output.shapes.len();
  let meshes = context.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
  let vertices: usize = meshes
    .iter()
    .map(|p| match &p.primitive {
      egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
      _ => 0,
    })
    .sum();
  output.textures_delta.clear();
  assert!(
    vertices < 1_000_000,
    "Подсветка перегружает видеопамять: {vertices} вершин"
  );
  commands
}

#[test]
fn dense_diagnostic_markers_do_not_create_unbounded_drawing_commands() {
  let item = crate::dxf_import::load_dxf(
    &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/measurement_demo.dxf"),
  )
  .unwrap();
  let report = DiagnosticReport {
    findings: (0..20_000)
      .map(|_| Finding {
        kind: IssueKind::OpenContour,
        marker: Marker::Point(Point::new(10.0, 10.0)),
        detail: "Плотные диагностические маркеры".into(),
      })
      .collect(),
  };
  let commands = draw_report(&item, &report);
  assert!(
    commands < 100,
    "Неограниченная подсветка: {commands} команд отрисовки"
  );
}

#[test]
#[ignore = "Нужен локальный DXF_HEAVY_FIXTURE; производственный чертёж не публикуется"]
fn heavy_reference_diagnostics_have_a_bounded_render_frame() {
  let path = std::env::var_os("DXF_HEAVY_FIXTURE").expect("Не задан DXF_HEAVY_FIXTURE");
  let before = std::fs::read(&path).unwrap();
  let started = std::time::Instant::now();
  let item = crate::dxf_import::load_dxf(std::path::Path::new(&path)).unwrap();
  let report = analyze(&item);
  let commands = draw_report(&item, &report);
  println!(
    "Элементов: {}; замечаний: {}; команд: {commands}; время: {:?}",
    item.primitives.len(),
    report.findings.len(),
    started.elapsed()
  );
  assert!(
    commands < 20_000,
    "Превышен безопасный объём подсветки: {commands}"
  );
  assert_eq!(before, std::fs::read(path).unwrap());
}

#[test]
#[ignore = "Нужен локальный DXF_HEAVY_FIXTURE; производственный чертёж не публикуется"]
fn heavy_reference_individual_elements_have_area_and_perimeter() {
  use crate::geometry::{MeasureCurve, Primitive};
  let path = std::env::var_os("DXF_HEAVY_FIXTURE").expect("Не задан DXF_HEAVY_FIXTURE");
  let before = std::fs::read(&path).unwrap();
  let item = crate::dxf_import::load_dxf(std::path::Path::new(&path)).unwrap();
  let (index, round) = item
    .primitives
    .iter()
    .enumerate()
    .find_map(|(index, p)| {
      if !item.appearance.primitive_diagnostic(index) {
        return None;
      }
      let Primitive::Path { curves, .. } = p else {
        return None;
      };
      match curves.as_slice() {
        [MeasureCurve::Round(r)] if r.is_full() && !r.approximate && r.radius > 50.0 => {
          Some((index, *r))
        }
        _ => None,
      }
    })
    .expect("Не найдена контрольная окружность");
  let started = std::time::Instant::now();
  let result = crate::region::measure_contour(&item, round.center).unwrap();
  println!(
    "Контур {index}: центр {:?}, R = {}; S = {}; P = {}; время {:?}",
    round.center,
    round.radius,
    result.area,
    result.perimeter,
    started.elapsed()
  );
  assert!((result.area - std::f64::consts::PI * round.radius.powi(2)).abs() < 1e-5);
  assert!((result.perimeter - std::f64::consts::TAU * round.radius).abs() < 1e-5);
  assert_eq!(before, std::fs::read(path).unwrap());
}

#[test]
#[ignore = "Нужен локальный DXF_HEAVY_FIXTURE; производственный чертёж не публикуется"]
fn heavy_reference_hexagonal_ceiling_elements_are_measurable() {
  use crate::geometry::Primitive;
  let path = std::env::var_os("DXF_HEAVY_FIXTURE").expect("Не задан DXF_HEAVY_FIXTURE");
  let item = crate::dxf_import::load_dxf(std::path::Path::new(&path)).unwrap();
  let mut hexagons = Vec::new();
  let mut errors = Vec::new();
  for (index, primitive) in item.primitives.iter().enumerate() {
    let Primitive::Path { points, closed, .. } = primitive else {
      continue;
    };
    if !*closed || !item.appearance.primitive_visible(index) {
      continue;
    }
    let points = if points.len() == 7 && points[0] == points[6] {
      &points[..6]
    } else {
      &points[..]
    };
    if points.len() != 6 {
      continue;
    }
    let center = Point::new(
      points.iter().map(|p| p.x).sum::<f64>() / 6.0,
      points.iter().map(|p| p.y).sum::<f64>() / 6.0,
    );
    let lengths: Vec<_> = (0..6)
      .map(|i| crate::planar::distance(points[i], points[(i + 1) % 6]))
      .collect();
    let low = lengths.iter().copied().fold(f64::INFINITY, f64::min);
    let high = lengths.iter().copied().fold(0.0, f64::max);
    if low < 100.0 || high / low > 1.01 {
      continue;
    }
    hexagons.push((index, points, center, lengths[0]));
  }
  let checked = hexagons.len().min(20);
  let started = std::time::Instant::now();
  for &(index, _, pick, _) in hexagons.iter().take(checked) {
    // В чертеже есть вложенные шестиугольники. Независимо находим меньший по шести полуплоскостям.
    let expected_side = hexagons
      .iter()
      .filter(|(_, points, _, _)| {
        let crosses: Vec<_> = (0..6)
          .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % 6];
            (b.x - a.x) * (pick.y - a.y) - (b.y - a.y) * (pick.x - a.x)
          })
          .collect();
        crosses.iter().all(|v| *v >= -1e-6) || crosses.iter().all(|v| *v <= 1e-6)
      })
      .map(|(_, _, _, side)| *side)
      .min_by(f64::total_cmp)
      .unwrap();
    match crate::region::measure_contour(&item, pick) {
      Ok(result) => {
        let expected = 3.0 * 3.0_f64.sqrt() / 2.0 * expected_side.powi(2);
        if (result.area - expected).abs() > expected * 0.00001 {
          errors.push(format!(
            "{index}: неверная площадь {} вместо {expected}",
            result.area
          ));
        }
        let perimeter = expected_side * 6.0;
        if (result.perimeter - perimeter).abs() >= perimeter * 0.00001 {
          errors.push(format!(
            "{index}: неверный периметр {} вместо {perimeter}; точка {pick:?}",
            result.perimeter
          ));
        }
      }
      Err(error) => errors.push(format!(
        "{index}, центр {pick:?}, диагностический {}: {error}",
        item.appearance.primitive_diagnostic(index)
      )),
    }
  }
  assert!(checked > 0, "Не найдены контрольные шестиугольники");
  println!(
    "Проверено шестиугольников: {checked}, отказов: {}; время расчётов {:?}",
    errors.len(),
    started.elapsed()
  );
  assert!(errors.is_empty(), "{}", errors.join("\n"));
}
