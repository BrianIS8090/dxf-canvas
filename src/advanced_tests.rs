use crate::{
  diagnostics::{DiagnosticsState, IssueKind, Marker, analyze},
  geometry::{DrawingItem, Point, ViewTransform},
  measurement::{DimensionKind, MeasurementState, Tool},
  region::measure_region,
};
use dxf::{
  Drawing,
  entities::{Arc, Circle, Entity, EntityType, Line},
  enums::Units,
};

fn drawing() -> Drawing {
  let mut drawing = Drawing::new();
  drawing.header.version = dxf::enums::AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;
  drawing
}

fn imported(drawing: Drawing) -> DrawingItem {
  let directory = tempfile::tempdir().unwrap();
  let path = directory.path().join("advanced.dxf");
  drawing.save_file(&path).unwrap();
  crate::dxf_import::load_dxf(&path).unwrap()
}

fn line(drawing: &mut Drawing, a: (f64, f64), b: (f64, f64)) {
  drawing.add_entity(Entity::new(EntityType::Line(Line::new(
    dxf::Point::new(a.0, a.1, 0.0),
    dxf::Point::new(b.0, b.1, 0.0),
  ))));
}

fn rectangle(drawing: &mut Drawing, x: f64, y: f64, w: f64, h: f64) {
  // Намеренно разный порядок и направление исходных отрезков.
  for (a, b) in [
    ((x, y), (x + w, y)),
    ((x, y + h), (x + w, y + h)),
    ((x, y), (x, y + h)),
    ((x + w, y + h), (x + w, y)),
  ] {
    line(drawing, a, b);
  }
}

fn circle(drawing: &mut Drawing, x: f64, y: f64, radius: f64) {
  drawing.add_entity(Entity::new(EntityType::Circle(Circle {
    center: dxf::Point::new(x, y, 0.0),
    radius,
    ..Default::default()
  })));
}

fn near(actual: f64, expected: f64) {
  assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

#[test]
fn boundary_relief_slit_adds_one_cut_pass_without_changing_area() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  line(&mut d, (100.0, 25.0), (99.0, 25.0));
  let item = imported(d);
  let result = measure_region(&item, Point::new(5.0, 5.0)).unwrap();
  near(result.area, 5000.0);
  near(result.perimeter, 301.0);
  near(result.slit_length, 1.0);
  assert_eq!(result.slit_count, 1);
  assert!(!result.approximate);
}

#[test]
fn connected_relief_segments_at_a_corner_are_counted_once() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  line(&mut d, (6.0, 4.0), (3.0, 4.0));
  line(&mut d, (0.0, 0.0), (3.0, 4.0));
  let result = measure_region(&imported(d), Point::new(10.0, 10.0)).unwrap();
  near(result.area, 5000.0);
  near(result.perimeter, 308.0);
  near(result.slit_length, 8.0);
  assert_eq!(result.slit_count, 1);
  assert_eq!(result.slits.len(), 2);
}

#[test]
fn relief_slits_may_start_on_a_hole_or_follow_an_arc() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  circle(&mut d, 25.0, 25.0, 10.0);
  line(&mut d, (25.0, 36.0), (25.0, 35.0));
  d.add_entity(Entity::new(EntityType::Arc(Arc {
    center: dxf::Point::new(98.0, 20.0, 0.0),
    radius: 2.0,
    start_angle: 0.0,
    end_angle: 90.0,
    ..Default::default()
  })));
  let result = measure_region(&imported(d), Point::new(5.0, 5.0)).unwrap();
  near(result.area, 5000.0 - 100.0 * std::f64::consts::PI);
  near(result.perimeter, 301.0 + 21.0 * std::f64::consts::PI);
  near(result.slit_length, 1.0 + std::f64::consts::PI);
  assert_eq!(result.slit_count, 2);
  assert_eq!(result.holes, 1);
  assert!(!result.approximate);
}

#[test]
fn relief_slits_do_not_hide_crossings_branches_or_overlaps() {
  let cases = [
    vec![((100.0, 25.0), (101.0, 25.0))],
    vec![((100.0, 25.0), (0.0, 25.0))],
    vec![((100.0, 25.0), (90.0, 25.0)), ((95.0, 50.0), (95.0, 20.0))],
    vec![((100.0, 25.0), (90.0, 25.0)), ((90.0, 25.0), (100.0, 25.0))],
    vec![((100.0, 25.0), (90.0, 25.0)), ((100.0, 25.0), (95.0, 25.0))],
    vec![
      ((0.0, 0.0), (3.0, 4.0)),
      ((3.0, 4.0), (6.0, 4.0)),
      ((3.0, 4.0), (3.0, 8.0)),
    ],
  ];
  for segments in cases {
    let mut d = drawing();
    rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
    for &(a, b) in &segments {
      line(&mut d, a, b);
    }
    assert!(
      measure_region(&imported(d), Point::new(5.0, 10.0)).is_err(),
      "Неоднозначная прорезь: {segments:?}"
    );
  }
}

#[test]
fn relief_slits_cannot_pass_through_holes_or_end_inside_them() {
  for end in [(25.0, 25.0), (10.0, 25.0)] {
    let mut d = drawing();
    rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
    circle(&mut d, 25.0, 25.0, 10.0);
    line(&mut d, (100.0, 25.0), end);
    assert!(measure_region(&imported(d), Point::new(5.0, 5.0)).is_err());
  }
}

#[test]
fn hidden_relief_slits_are_excluded_and_other_parts_are_independent() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  line(&mut d, (100.0, 25.0), (99.0, 25.0));
  rectangle(&mut d, 200.0, 0.0, 100.0, 50.0);
  line(&mut d, (300.0, 25.0), (299.0, 25.0));
  let mut item = imported(d);
  let result = measure_region(&item, Point::new(5.0, 5.0)).unwrap();
  near(result.perimeter, 301.0);
  assert_eq!(result.slit_count, 1);
  item.appearance.styles[4].visible = false;
  let result = measure_region(&item, Point::new(5.0, 5.0)).unwrap();
  near(result.perimeter, 300.0);
  assert_eq!(result.slit_count, 0);
}

#[test]
fn relief_slit_preview_and_placed_label_use_source_units_not_visual_scale() {
  let mut d = drawing();
  d.header.default_drawing_units = Units::Inches;
  rectangle(&mut d, 0.0, 0.0, 2.0, 1.0);
  line(&mut d, (2.0, 0.5), (1.5, 0.5));
  let mut item = imported(d);
  item.scale = 3.0;
  item.offset = Point::new(10.0, 20.0);
  let view = ViewTransform {
    scale: 100.0,
    origin: eframe::egui::pos2(0.0, 500.0),
  };
  let screen = |p| view.world_to_screen(item.world_point(p));
  let mut state = MeasurementState::default();
  state.set_tool(Tool::Region);
  state.click(
    std::slice::from_ref(&item),
    view,
    screen(Point::new(0.5, 0.5)),
  );
  assert!(state.notice.is_none());
  let label = screen(Point::new(3.0, 2.0));
  let preview = state
    .preview(std::slice::from_ref(&item), view, label)
    .unwrap();
  let text = preview.text(&item);
  assert!(text.contains("1290,32 мм²"), "{text}");
  assert!(text.contains("165,1 мм"), "{text}");
  assert!(
    text.contains("Прорези: 1 · 12,7 мм (включены в P)"),
    "{text}"
  );
  let DimensionKind::Region(region) = &preview.kind else {
    panic!("Нет площади")
  };
  assert_eq!(region.slits.len(), 1);
  state.click(std::slice::from_ref(&item), view, label);
  assert_eq!(state.completed.len(), 1);
  assert_eq!(state.completed[0].text(&item), text);
  state.undo();
  assert!(state.completed.is_empty());
}

#[test]
#[ignore = "Нужен локальный DXF_AREA_FIXTURE; производственный чертёж не публикуется"]
fn supplied_bracket_has_area_and_perimeter() {
  let path = std::env::var_os("DXF_AREA_FIXTURE").expect("Не задан DXF_AREA_FIXTURE");
  let before = std::fs::read(&path).unwrap();
  let item = crate::dxf_import::load_dxf(std::path::Path::new(&path)).unwrap();
  let point = Point::new(
    (item.bounds.min.x + item.bounds.max.x) * 0.5,
    (item.bounds.min.y + item.bounds.max.y) * 0.5,
  );
  let result = measure_region(&item, point).unwrap();
  near(result.area, 1750.050748721657);
  near(result.perimeter, 289.83275653839775);
  near(result.slit_length, 2.0);
  assert_eq!(result.slit_count, 2);
  assert_eq!(result.holes, 5);
  assert!(!result.approximate);
  assert_eq!(std::fs::read(&path).unwrap(), before);
  println!(
    "S = {:.9}; P = {:.9}; отверстий: {}; прорезей: {}; длина прорезей: {:.9}",
    result.area, result.perimeter, result.holes, result.slit_count, result.slit_length
  );
}

#[test]
fn area_of_separate_lines_subtracts_holes_and_includes_their_perimeters() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  circle(&mut d, 25.0, 25.0, 10.0);
  let item = imported(d);
  let result = measure_region(&item, Point::new(5.0, 5.0)).unwrap();
  near(result.area, 5000.0 - 100.0 * std::f64::consts::PI);
  near(result.perimeter, 300.0 + 20.0 * std::f64::consts::PI);
  assert_eq!(result.holes, 1);
  assert!(!result.approximate);
}

#[test]
fn rounded_rectangle_uses_exact_arc_integrals_after_visual_scaling() {
  let mut item = crate::dxf_import::load_dxf(
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
      .join("examples/measurement_demo.dxf")
      .as_path(),
  )
  .unwrap();
  item.offset = Point::new(5000.0, -2500.0);
  item.scale = 5.0;
  let result = measure_region(&item, Point::new(20.0, 20.0)).unwrap();
  near(result.area, 15600.0 + 48.0 * std::f64::consts::PI);
  near(result.perimeter, 440.0 + 40.0 * std::f64::consts::PI);
  assert_eq!(result.holes, 2);
  assert!(!result.approximate);
}

#[test]
fn nested_islands_are_added_back_and_disjoint_parts_are_not_combined() {
  let mut d = drawing();
  for radius in [10.0, 5.0, 2.0] {
    circle(&mut d, 0.0, 0.0, radius);
  }
  rectangle(&mut d, 100.0, 100.0, 20.0, 10.0);
  let item = imported(d);
  let ring = measure_region(&item, Point::new(8.0, 0.0)).unwrap();
  near(ring.area, 79.0 * std::f64::consts::PI);
  near(ring.perimeter, 34.0 * std::f64::consts::PI);
  assert_eq!(ring.holes, 1);
  let rectangle = measure_region(&item, Point::new(105.0, 105.0)).unwrap();
  near(rectangle.area, 200.0);
  near(rectangle.perimeter, 60.0);
}

#[test]
fn area_does_not_silently_ignore_open_inner_contours_or_duplicates() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  line(&mut d, (20.0, 20.0), (30.0, 20.0));
  assert!(measure_region(&imported(d), Point::new(5.0, 5.0)).is_err());
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  circle(&mut d, 25.0, 25.0, 10.0);
  circle(&mut d, 25.0, 25.0, 10.0);
  assert!(measure_region(&imported(d), Point::new(5.0, 5.0)).is_err());
}

#[test]
fn intersecting_and_tangent_holes_are_not_given_a_misleading_area() {
  for x in [35.0, 45.0] {
    let mut d = drawing();
    rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
    circle(&mut d, 25.0, 25.0, 10.0);
    circle(&mut d, x, 25.0, 10.0);
    assert!(measure_region(&imported(d), Point::new(5.0, 5.0)).is_err());
  }
}

#[test]
fn area_tool_places_a_dimension_converts_square_units_and_undoes_it() {
  let mut d = drawing();
  d.header.default_drawing_units = Units::Inches;
  rectangle(&mut d, 0.0, 0.0, 2.0, 1.0);
  let item = imported(d);
  let view = ViewTransform {
    scale: 100.0,
    origin: eframe::egui::pos2(0.0, 500.0),
  };
  let screen = |p| view.world_to_screen(item.world_point(p));
  let mut state = MeasurementState::default();
  state.set_tool(Tool::Region);
  state.click(
    std::slice::from_ref(&item),
    view,
    screen(Point::new(0.5, 0.5)),
  );
  assert!(state.notice.is_none());
  let preview = state
    .preview(
      std::slice::from_ref(&item),
      view,
      screen(Point::new(3.0, 2.0)),
    )
    .unwrap();
  assert!(preview.text(&item).contains("1290,32 мм²"));
  assert!(preview.text(&item).contains("152,4 мм"));
  state.click(
    std::slice::from_ref(&item),
    view,
    screen(Point::new(3.0, 2.0)),
  );
  assert_eq!(state.completed.len(), 1);
  state.undo();
  assert!(state.completed.is_empty());
}

#[test]
fn angular_tool_uses_three_snapped_points_then_places_the_smaller_angle() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  let mut item = imported(d);
  item.scale = 3.0;
  item.offset = Point::new(800.0, 1200.0);
  let view = ViewTransform {
    scale: 2.0,
    origin: eframe::egui::pos2(0.0, 800.0),
  };
  let screen = |p| view.world_to_screen(item.world_point(p));
  let mut state = MeasurementState::default();
  state.set_tool(Tool::Angle);
  for p in [
    Point::new(100.0, 0.0),
    Point::new(0.0, 0.0),
    Point::new(0.0, 50.0),
    Point::new(30.0, 30.0),
  ] {
    state.click(std::slice::from_ref(&item), view, screen(p));
  }
  assert_eq!(state.completed.len(), 1);
  assert!(matches!(
    state.completed[0].kind,
    DimensionKind::Angle { .. }
  ));
  assert_eq!(state.completed[0].text(&item), "90°");
  state.undo();
  assert!(state.completed.is_empty());
}

#[test]
fn crossings_and_t_junctions_have_point_markers_but_normal_joints_do_not() {
  let mut d = drawing();
  line(&mut d, (0.0, 0.0), (100.0, 100.0));
  line(&mut d, (0.0, 100.0), (100.0, 0.0));
  line(&mut d, (25.0, 25.0), (25.0, 0.0));
  let report = analyze(&imported(d));
  assert_eq!(report.count(IssueKind::Intersection), 2);
  assert!(
    report
      .findings
      .iter()
      .any(|f| f.kind == IssueKind::Intersection
        && matches!(f.marker, Marker::Point(p) if distance(p, Point::new(50.0, 50.0)) < 1e-8))
  );
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  assert_eq!(analyze(&imported(d)).count(IssueKind::Intersection), 0);
}

fn distance(a: Point, b: Point) -> f64 {
  (a.x - b.x).hypot(a.y - b.y)
}

#[test]
fn partial_line_overlap_focuses_only_the_shared_section() {
  let mut d = drawing();
  line(&mut d, (0.0, 0.0), (100.0, 0.0));
  line(&mut d, (60.0, 0.0), (30.0, 0.0));
  let item = imported(d);
  let report = analyze(&item);
  assert_eq!(report.count(IssueKind::PartialOverlap), 1);
  let b = report
    .findings
    .iter()
    .find(|f| f.kind == IssueKind::PartialOverlap)
    .unwrap()
    .marker
    .bounds(&item)
    .unwrap();
  near(b.min.x, 30.0);
  near(b.max.x, 60.0);
}

#[test]
fn circular_arcs_report_partial_overlap_without_flattening() {
  let mut d = drawing();
  for (start_angle, end_angle) in [(0.0, 180.0), (90.0, 270.0)] {
    d.add_entity(Entity::new(EntityType::Arc(Arc {
      radius: 10.0,
      start_angle,
      end_angle,
      ..Default::default()
    })));
  }
  let item = imported(d);
  let report = analyze(&item);
  assert_eq!(report.count(IssueKind::PartialOverlap), 1);
  let b = report
    .findings
    .iter()
    .find(|f| f.kind == IssueKind::PartialOverlap)
    .unwrap()
    .marker
    .bounds(&item)
    .unwrap();
  near(b.min.x, -10.0);
  near(b.max.x, 0.0);
  near(b.min.y, 0.0);
  near(b.max.y, 10.0);
}

#[test]
fn line_circle_and_circle_circle_intersections_are_located_analytically() {
  let mut d = drawing();
  circle(&mut d, 0.0, 0.0, 10.0);
  line(&mut d, (-20.0, 0.0), (20.0, 0.0));
  let report = analyze(&imported(d));
  assert_eq!(report.count(IssueKind::Intersection), 2);
  let mut d = drawing();
  circle(&mut d, 0.0, 0.0, 10.0);
  circle(&mut d, 10.0, 0.0, 10.0);
  assert_eq!(analyze(&imported(d)).count(IssueKind::Intersection), 2);
}

#[test]
fn self_crossing_polyline_is_detected_and_area_is_refused() {
  let mut d = drawing();
  let mut polyline = dxf::entities::LwPolyline::default();
  for (x, y) in [(0.0, 0.0), (100.0, 100.0), (0.0, 100.0), (100.0, 0.0)] {
    polyline.vertices.push(dxf::LwPolylineVertex {
      x,
      y,
      ..Default::default()
    });
  }
  polyline.set_is_closed(true);
  d.add_entity(Entity::new(EntityType::LwPolyline(polyline)));
  let item = imported(d);
  assert_eq!(analyze(&item).count(IssueKind::Intersection), 1);
  assert!(measure_region(&item, Point::new(50.0, 10.0)).is_err());
}

#[test]
fn navigation_wraps_across_files_and_obeys_filter_and_toggle() {
  let mut d = drawing();
  line(&mut d, (0.0, 0.0), (100.0, 100.0));
  line(&mut d, (0.0, 100.0), (100.0, 0.0));
  let item = imported(d);
  let mut state = DiagnosticsState::default();
  state.toggle(&[item.clone(), item.clone()]);
  state.set_filter(Some(IssueKind::Intersection));
  assert_eq!(state.visible_selections().len(), 2);
  assert!(state.navigate(true));
  assert_eq!(state.selected.unwrap().item, 0);
  assert!(state.take_focus_request().is_some());
  state.navigate(true);
  assert_eq!(state.selected.unwrap().item, 1);
  state.navigate(true);
  assert_eq!(state.selected.unwrap().item, 0);
  state.navigate(false);
  assert_eq!(state.selected.unwrap().item, 1);
  state.set_filter(Some(IssueKind::PartialOverlap));
  assert!(state.selected.is_none());
  assert!(state.take_focus_request().is_none());
  assert!(!state.navigate(true));
  state.toggle(&[item]);
  assert!(state.visible_selections().is_empty());
}

#[test]
fn hidden_layers_are_excluded_from_area_and_new_diagnostics() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  line(&mut d, (0.0, 0.0), (100.0, 50.0));
  line(&mut d, (0.0, 50.0), (100.0, 0.0));
  let mut item = imported(d);
  for style in &mut item.appearance.styles[4..] {
    style.visible = false;
  }
  assert_eq!(analyze(&item).count(IssueKind::Intersection), 0);
  near(
    measure_region(&item, Point::new(5.0, 5.0)).unwrap().area,
    5000.0,
  );
}

#[test]
fn area_is_translation_invariant_in_large_source_coordinates() {
  let mut d = drawing();
  rectangle(&mut d, 1_000_000_000.0, -1_000_000_000.0, 100.0, 50.0);
  let item = imported(d);
  let area = measure_region(&item, Point::new(1_000_000_010.0, -999_999_990.0)).unwrap();
  near(area.area, 5000.0);
  near(area.perimeter, 300.0);
}

#[test]
fn diagnostics_filter_hides_other_categories_on_the_canvas() {
  let mut d = drawing();
  line(&mut d, (0.0, 0.0), (100.0, 100.0));
  line(&mut d, (0.0, 100.0), (100.0, 0.0));
  let item = imported(d);
  let report = analyze(&item);
  let context = eframe::egui::Context::default();
  let mut output = context.run_ui(Default::default(), |ui| {
    crate::diagnostics_ui::paint_report(
      ui.painter(),
      &item,
      &report,
      ViewTransform {
        scale: 1.0,
        origin: eframe::egui::pos2(0.0, 100.0),
      },
      Some(IssueKind::PartialOverlap),
    );
  });
  output.textures_delta.clear();
  assert!(
    output
      .shapes
      .iter()
      .all(|s| matches!(s.shape, eframe::egui::Shape::Noop))
  );
}

#[test]
fn sampled_contour_area_is_explicitly_marked_as_approximate() {
  let mut d = drawing();
  rectangle(&mut d, 0.0, 0.0, 100.0, 50.0);
  let mut item = imported(d);
  let points = vec![
    Point::new(0.0, 0.0),
    Point::new(100.0, 0.0),
    Point::new(100.0, 50.0),
    Point::new(0.0, 50.0),
  ];
  item.primitives = vec![crate::geometry::Primitive::Path {
    points: points.clone(),
    closed: true,
    curves: vec![crate::geometry::MeasureCurve::Polyline {
      points,
      closed: true,
    }],
  }];
  let region = measure_region(&item, Point::new(5.0, 5.0)).unwrap();
  assert!(region.approximate);
  near(region.area, 5000.0);
}
