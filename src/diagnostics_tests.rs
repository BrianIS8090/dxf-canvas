use crate::{
  diagnostics::{DiagnosticsState, IssueKind, Marker, analyze},
  geometry::{DrawingItem, Point},
};
use dxf::{
  Drawing,
  entities::{Arc, Circle, Ellipse, Entity, EntityType, Line, LwPolyline},
  enums::{AcadVersion, Units},
};

fn drawing() -> Drawing {
  let mut drawing = Drawing::new();
  drawing.header.version = AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;
  drawing
}

fn imported(drawing: Drawing) -> DrawingItem {
  let stamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_nanos();
  let path = std::env::temp_dir().join(format!(
    "dxf_diagnostics_{}_{stamp}.dxf",
    std::process::id()
  ));
  drawing.save_file(&path).unwrap();
  let result = crate::dxf_import::load_dxf(&path).unwrap();
  std::fs::remove_file(path).unwrap();
  result
}

fn line(drawing: &mut Drawing, a: (f64, f64), b: (f64, f64)) {
  drawing.add_entity(Entity::new(EntityType::Line(Line::new(
    dxf::Point::new(a.0, a.1, 0.0),
    dxf::Point::new(b.0, b.1, 0.0),
  ))));
}

fn circle(drawing: &mut Drawing, center: (f64, f64), radius: f64) {
  drawing.add_entity(Entity::new(EntityType::Circle(Circle {
    center: dxf::Point::new(center.0, center.1, 0.0),
    radius,
    ..Default::default()
  })));
}

fn rectangle() -> Drawing {
  let mut drawing = drawing();
  for (a, b) in [
    ((0.0, 0.0), (100.0, 0.0)),
    ((100.0, 0.0), (100.0, 50.0)),
    ((100.0, 50.0), (0.0, 50.0)),
    ((0.0, 50.0), (0.0, 0.0)),
  ] {
    line(&mut drawing, a, b);
  }
  drawing
}

#[test]
fn closed_contour_assembled_from_lines_is_not_reported_as_open() {
  let report = analyze(&imported(rectangle()));
  assert_eq!(report.count(IssueKind::OpenContour), 0);
  assert_eq!(report.count(IssueKind::UnjoinedContour), 1);
  assert_eq!(report.findings.len(), 1);
}

#[test]
fn open_chain_highlights_only_its_two_free_ends() {
  let mut drawing = drawing();
  line(&mut drawing, (0.0, 0.0), (100.0, 0.0));
  line(&mut drawing, (100.0, 0.0), (100.0, 50.0));
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::OpenContour), 2);
  assert!(report.findings.iter().all(|finding| matches!(finding.marker, Marker::Point(p) if p == Point::new(0.0, 0.0) || p == Point::new(100.0, 50.0))));
}

#[test]
fn reversed_duplicate_lines_do_not_hide_their_open_ends() {
  let mut drawing = drawing();
  line(&mut drawing, (0.0, 0.0), (100.0, 0.0));
  line(&mut drawing, (100.0, 0.0), (0.0, 0.0));
  circle(&mut drawing, (50.0, 20.0), 5.0);
  circle(&mut drawing, (50.0, 20.0), 5.0);
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::Duplicate), 2);
  assert_eq!(report.count(IssueKind::OpenContour), 2);
}

#[test]
fn two_opposite_semicircles_are_a_valid_closed_contour_not_duplicates() {
  let mut drawing = drawing();
  for start in [0.0, 180.0] {
    drawing.add_entity(Entity::new(EntityType::Arc(Arc {
      radius: 20.0,
      start_angle: start,
      end_angle: start + 180.0,
      ..Default::default()
    })));
  }
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::OpenContour), 0);
  assert_eq!(report.count(IssueKind::Duplicate), 0);
  assert_eq!(report.count(IssueKind::UnjoinedContour), 1);
}

#[test]
fn short_real_segments_are_reported_but_circle_display_segments_are_not() {
  let mut drawing = drawing();
  circle(&mut drawing, (50.0, 50.0), 0.5);
  line(&mut drawing, (5.0, 5.0), (5.0, 5.0));
  drawing.add_entity(Entity::new(EntityType::LwPolyline(LwPolyline {
    vertices: [(0.0, 0.0), (0.05, 0.0), (100.0, 0.0)]
      .into_iter()
      .map(|(x, y)| dxf::LwPolylineVertex {
        x,
        y,
        ..Default::default()
      })
      .collect(),
    ..Default::default()
  })));
  assert_eq!(
    analyze(&imported(drawing)).count(IssueKind::ShortSegment),
    2
  );
}

#[test]
fn rotated_oval_is_warned_about_but_round_contours_are_not() {
  let mut drawing = drawing();
  for ratio in [0.98, 1.0] {
    drawing.add_entity(Entity::new(EntityType::Ellipse(Ellipse {
      center: dxf::Point::new(1000.0, -2000.0, 0.0),
      major_axis: dxf::Vector::new(10.0, 10.0, 0.0),
      minor_axis_ratio: ratio,
      end_parameter: std::f64::consts::TAU,
      ..Default::default()
    })));
  }
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::Oval), 1);
  assert_eq!(report.count(IssueKind::OpenContour), 0);
}

#[test]
fn closed_polygon_is_not_misidentified_as_an_oval() {
  let mut drawing = drawing();
  let mut vertices = Vec::new();
  for i in 0..8 {
    vertices.push(dxf::LwPolylineVertex {
      x: i as f64 * 10.0,
      y: 0.0,
      ..Default::default()
    });
  }
  for i in 0..8 {
    vertices.push(dxf::LwPolylineVertex {
      x: 80.0,
      y: i as f64 * 10.0,
      ..Default::default()
    });
  }
  for i in 0..8 {
    vertices.push(dxf::LwPolylineVertex {
      x: 80.0 - i as f64 * 10.0,
      y: 80.0,
      ..Default::default()
    });
  }
  for i in 0..8 {
    vertices.push(dxf::LwPolylineVertex {
      x: 0.0,
      y: 80.0 - i as f64 * 10.0,
      ..Default::default()
    });
  }
  let mut outline = LwPolyline {
    vertices,
    ..Default::default()
  };
  outline.set_is_closed(true);
  drawing.add_entity(Entity::new(EntityType::LwPolyline(outline)));
  assert!(analyze(&imported(drawing)).findings.is_empty());
}

#[test]
fn toggle_removes_reports_and_does_not_change_source_geometry() {
  let mut drawing = drawing();
  line(&mut drawing, (0.0, 0.0), (20.0, 0.0));
  let items = vec![imported(drawing)];
  let before = format!("{items:?}");
  let mut state = DiagnosticsState::default();
  assert!(!state.enabled);
  state.toggle(&items);
  assert!(state.enabled);
  assert_eq!(state.reports[0].count(IssueKind::OpenContour), 2);
  state.toggle(&items);
  assert!(!state.enabled);
  assert!(state.reports.is_empty());
  assert_eq!(before, format!("{items:?}"));
}

#[test]
fn warnings_stay_in_source_coordinates_when_the_detail_is_moved_and_scaled() {
  let mut drawing = drawing();
  line(&mut drawing, (0.0, 0.0), (100.0, 0.0));
  let mut item = imported(drawing);
  let expected = format!("{:?}", analyze(&item));
  item.scale = 7.0;
  item.offset = Point::new(500.0, -2000.0);
  assert_eq!(expected, format!("{:?}", analyze(&item)));
}

#[test]
fn adding_or_removing_files_updates_reports_without_joining_separate_files() {
  let mut drawing = drawing();
  line(&mut drawing, (0.0, 0.0), (20.0, 0.0));
  let item = imported(drawing);
  let mut items = vec![item.clone()];
  let mut state = DiagnosticsState::default();
  state.toggle(&items);
  items.push(item);
  state.refresh(&items);
  assert_eq!(state.reports.len(), 2);
  assert!(
    state
      .reports
      .iter()
      .all(|report| report.count(IssueKind::OpenContour) == 2)
  );
  items.remove(0);
  state.refresh(&items);
  assert_eq!(state.reports.len(), 1);
  state.clear();
  state.refresh(&items);
  assert!(!state.enabled);
  assert!(state.reports.is_empty());
}

#[test]
fn unknown_units_and_unsupported_entities_are_file_warnings() {
  let mut item = imported(rectangle());
  item.units = Default::default();
  item.unsupported_entities = 3;
  let report = analyze(&item);
  assert_eq!(report.count(IssueKind::UnknownUnits), 1);
  assert_eq!(report.count(IssueKind::IncompleteGeometry), 1);
  assert!(
    report
      .findings
      .iter()
      .filter(|finding| finding.kind != IssueKind::UnjoinedContour)
      .all(|finding| matches!(finding.marker, Marker::File))
  );
}

#[test]
fn length_threshold_is_in_millimeters_even_for_inch_drawings() {
  let mut drawing = drawing();
  drawing.header.default_drawing_units = Units::Inches;
  line(&mut drawing, (0.0, 0.0), (0.005, 0.0));
  line(&mut drawing, (1.0, 1.0), (1.002, 1.0));
  assert_eq!(
    analyze(&imported(drawing)).count(IssueKind::ShortSegment),
    1
  );
}

#[test]
fn endpoint_join_tolerance_is_respected() {
  for (gap, expected) in [(0.005, 2), (0.02, 4)] {
    let mut drawing = drawing();
    line(&mut drawing, (0.0, 0.0), (20.0, 0.0));
    line(&mut drawing, (20.0 + gap, 0.0), (40.0, 0.0));
    assert_eq!(
      analyze(&imported(drawing)).count(IssueKind::OpenContour),
      expected
    );
  }
}

fn problem_drawing() -> Drawing {
  // Локальную регрессию можно повторить на закрытом чертеже, не добавляя его в Git.
  if let Some(path) = std::env::var_os("DXF_PRIVATE_REGRESSION_FIXTURE") {
    Drawing::load_file(path).unwrap()
  } else {
    crate::test_fixtures::diagnostics_drawing()
  }
}

pub(crate) fn problem_areas() -> DrawingItem {
  imported(problem_drawing())
}

fn marks_primitive(marker: &Marker, index: usize) -> bool {
  match marker {
    Marker::Primitive(primitive) | Marker::Curve { primitive, .. } => *primitive == index,
    Marker::Contour(primitives) => primitives.contains(&index),
    _ => false,
  }
}

fn problem_subset(indices: &[usize]) -> DrawingItem {
  let original = problem_drawing();
  let mut subset = drawing();
  for (index, entity) in original.entities().enumerate() {
    if indices.contains(&index) {
      subset.add_entity(entity.clone());
    }
  }
  imported(subset)
}

#[test]
fn minimized_supplied_rounded_rectangle_is_highlighted() {
  let report = analyze(&problem_subset(&(60..=67).collect::<Vec<_>>()));
  assert!((0..8).all(|index| {
    report
      .findings
      .iter()
      .any(|finding| marks_primitive(&finding.marker, index))
  }));
}

#[test]
fn minimized_supplied_deformed_hole_is_highlighted() {
  let report = analyze(&problem_subset(&[73]));
  assert!(
    report
      .findings
      .iter()
      .any(|finding| marks_primitive(&finding.marker, 0))
  );
}

#[test]
fn minimized_supplied_two_arc_circle_is_highlighted() {
  let report = analyze(&problem_subset(&[75, 76]));
  assert!((0..2).all(|index| {
    report
      .findings
      .iter()
      .any(|finding| marks_primitive(&finding.marker, index))
  }));
}

#[test]
fn supplied_rounded_rectangle_has_an_unjoined_warning() {
  let report = analyze(&problem_areas());
  assert!(
    (60..=67).all(|index| {
      report
        .findings
        .iter()
        .any(|finding| marks_primitive(&finding.marker, index))
    }),
    "Все восемь раздельных элементов скруглённого прямоугольника должны подсвечиваться"
  );
}

#[test]
fn supplied_deformed_round_hole_has_a_shape_warning() {
  let report = analyze(&problem_areas());
  assert!(
    report
      .findings
      .iter()
      .any(|finding| marks_primitive(&finding.marker, 73)),
    "Некруглое отверстие сверху кольца должно подсвечиваться"
  );
}

#[test]
fn supplied_two_arc_circle_has_an_unjoined_warning() {
  let report = analyze(&problem_areas());
  assert!(
    [75, 76].into_iter().all(|index| {
      report
        .findings
        .iter()
        .any(|finding| marks_primitive(&finding.marker, index))
    }),
    "Обе раздельные половины окружности должны подсвечиваться"
  );
}

#[test]
fn supplied_file_reports_exact_kinds_without_inventing_gaps() {
  let item = problem_areas();
  let report = analyze(&item);
  assert_eq!(report.count(IssueKind::UnjoinedContour), 2);
  assert_eq!(report.count(IssueKind::Oval), 2);
  assert_eq!(report.count(IssueKind::OpenContour), 4);
  assert_eq!(report.count(IssueKind::Duplicate), 4);
  assert_eq!(report.count(IssueKind::ShortSegment), 1);
  assert_eq!(report.findings.len(), 13);
  for index in [73, 74] {
    assert!(
      report
        .findings
        .iter()
        .any(|finding| finding.kind == IssueKind::Oval && marks_primitive(&finding.marker, index))
    );
  }
  for indices in [(60..=67).collect::<Vec<_>>(), vec![75, 76]] {
    assert!(
      report
        .findings
        .iter()
        .any(|finding| finding.kind == IssueKind::UnjoinedContour
          && matches!(&finding.marker, Marker::Contour(actual) if *actual == indices))
    );
  }
}

#[test]
fn removing_one_piece_turns_a_separate_loop_into_an_open_chain() {
  for indices in [(60..=67).collect::<Vec<_>>(), vec![75, 76]] {
    for removed in &indices {
      let subset: Vec<_> = indices
        .iter()
        .copied()
        .filter(|index| index != removed)
        .collect();
      let report = analyze(&problem_subset(&subset));
      assert_eq!(report.count(IssueKind::UnjoinedContour), 0);
      assert_eq!(report.count(IssueKind::OpenContour), 2);
    }
  }
}

#[test]
fn a_real_gap_between_arcs_remains_a_gap_not_a_separate_closed_loop() {
  let mut drawing = drawing();
  for (start_angle, end_angle) in [(0.0, 180.0), (181.0, 359.0)] {
    drawing.add_entity(Entity::new(EntityType::Arc(Arc {
      radius: 7.5,
      start_angle,
      end_angle,
      ..Default::default()
    })));
  }
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::OpenContour), 4);
  assert_eq!(report.count(IssueKind::UnjoinedContour), 0);
}

#[test]
fn a_branch_is_not_reported_as_a_simple_separate_closed_loop() {
  let mut drawing = rectangle();
  line(&mut drawing, (0.0, 0.0), (-10.0, 0.0));
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::UnjoinedContour), 0);
  assert_eq!(report.count(IssueKind::OpenContour), 1);
}

#[test]
fn joined_rounded_rectangle_and_circles_keep_a_clean_report() {
  let mut drawing = drawing();
  let mut outline = LwPolyline {
    vertices: [
      (35.0, 0.0, 0.0),
      (65.0, 0.0, (std::f64::consts::PI / 8.0).tan()),
      (100.0, 35.0, 0.0),
      (100.0, 65.0, (std::f64::consts::PI / 8.0).tan()),
      (65.0, 100.0, 0.0),
      (35.0, 100.0, (std::f64::consts::PI / 8.0).tan()),
      (0.0, 65.0, 0.0),
      (0.0, 35.0, (std::f64::consts::PI / 8.0).tan()),
    ]
    .into_iter()
    .map(|(x, y, bulge)| dxf::LwPolylineVertex {
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
  circle(&mut drawing, (50.0, 50.0), 10.0);
  assert!(analyze(&imported(drawing)).findings.is_empty());
}

#[test]
fn duplicated_edge_does_not_hide_the_separate_loop_or_create_a_gap() {
  let mut drawing = rectangle();
  line(&mut drawing, (100.0, 0.0), (0.0, 0.0));
  let report = analyze(&imported(drawing));
  assert_eq!(report.count(IssueKind::UnjoinedContour), 1);
  assert_eq!(report.count(IssueKind::Duplicate), 1);
  assert_eq!(report.count(IssueKind::OpenContour), 0);
}

#[test]
fn updated_warnings_survive_move_scale_and_toggle_without_modifying_geometry() {
  let mut item = problem_areas();
  let expected = format!("{:?}", analyze(&item));
  item.offset = Point::new(1000.0, -500.0);
  item.scale = 0.1;
  assert_eq!(expected, format!("{:?}", analyze(&item)));
  let before = format!("{item:?}");
  let mut state = DiagnosticsState::default();
  state.toggle(std::slice::from_ref(&item));
  assert_eq!(expected, format!("{:?}", state.reports[0]));
  state.toggle(std::slice::from_ref(&item));
  assert!(state.reports.is_empty());
  assert_eq!(before, format!("{item:?}"));
}

#[test]
fn every_supplied_finding_has_a_finite_focus_region() {
  let item = problem_areas();
  for finding in analyze(&item).findings {
    let bounds = finding.marker.focus_bounds(&item).unwrap();
    assert!(bounds.is_valid());
    assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
  }
}

#[test]
fn contour_focus_contains_all_parts_but_not_the_whole_file() {
  let item = problem_areas();
  let bounds = Marker::Contour((60..=67).collect()).bounds(&item).unwrap();
  assert!((bounds.width() - 128.0).abs() < 1.0e-7);
  assert!((bounds.height() - 52.0).abs() < 1.0e-7);
  assert!(bounds.width() < item.bounds.width() / 5.0);
  assert_eq!(Marker::File.bounds(&item), Some(item.bounds));
}

#[test]
fn arc_focus_uses_only_the_arc_and_includes_its_extreme_points() {
  let mut item = imported(rectangle());
  item.primitives = vec![crate::geometry::Primitive::Path {
    points: vec![],
    closed: false,
    curves: vec![crate::geometry::MeasureCurve::Round(
      crate::geometry::RoundCurve {
        center: Point::new(10.0, 20.0),
        radius: 5.0,
        start: std::f64::consts::FRAC_PI_4,
        sweep: std::f64::consts::FRAC_PI_2,
        approximate: false,
      },
    )],
  }];
  let bounds = Marker::Curve {
    primitive: 0,
    curve: 0,
  }
  .bounds(&item)
  .unwrap();
  assert!((bounds.max.y - 25.0).abs() < 1.0e-9);
  assert!((bounds.min.y - (20.0 + 5.0 / 2.0_f64.sqrt())).abs() < 1.0e-9);
  assert!(bounds.width() < 10.0);
}

#[test]
fn point_focus_is_centered_after_individual_move_and_scale() {
  let mut item = imported(rectangle());
  item.scale = 2.0;
  item.offset = Point::new(300.0, 200.0);
  let bounds = Marker::Point(Point::new(20.0, 30.0))
    .focus_bounds(&item)
    .unwrap();
  assert_eq!(bounds.center(), Point::new(290.0, 235.0));
  assert_eq!(bounds.width(), 4.0);
  assert_eq!(bounds.height(), 4.0);
}

#[test]
fn focus_neighborhood_uses_declared_units() {
  let mut item = imported(rectangle());
  item.units = crate::geometry::LengthUnit::from_dxf_code(1);
  let bounds = Marker::Point(Point::new(0.0, 0.0))
    .focus_bounds(&item)
    .unwrap();
  assert!((bounds.width() * 25.4 - 2.0).abs() < 1.0e-10);
  item.units = Default::default();
  assert_eq!(
    Marker::Point(Point::default())
      .focus_bounds(&item)
      .unwrap()
      .width(),
    2.0
  );
}

#[test]
fn invalid_marker_indices_do_not_focus_an_unrelated_place() {
  let item = problem_areas();
  for marker in [
    Marker::Primitive(usize::MAX),
    Marker::Curve {
      primitive: 0,
      curve: usize::MAX,
    },
    Marker::Contour(vec![0, usize::MAX]),
    Marker::Contour(vec![]),
  ] {
    assert!(marker.focus_bounds(&item).is_none());
  }
}

#[test]
fn selecting_a_finding_requests_focus_once_and_again_on_repeat_click() {
  let item = problem_areas();
  let mut state = DiagnosticsState::default();
  assert!(!state.select(0, 0));
  state.toggle(std::slice::from_ref(&item));
  assert!(state.select(0, 11));
  let expected = crate::diagnostics::DiagnosticSelection {
    item: 0,
    finding: 11,
  };
  assert_eq!(state.selected, Some(expected));
  assert_eq!(state.take_focus_request(), Some(expected));
  assert_eq!(state.take_focus_request(), None);
  assert!(state.select(0, 11));
  assert_eq!(state.take_focus_request(), Some(expected));
  assert!(!state.select(5, 0));
  assert!(!state.select(0, 999));
  assert_eq!(state.selected, Some(expected));
}

#[test]
fn hiding_or_refreshing_diagnostics_clears_selection_and_pending_focus() {
  let items = vec![problem_areas()];
  let mut state = DiagnosticsState::default();
  state.toggle(&items);
  assert!(state.select(0, 0));
  state.toggle(&items);
  assert!(state.selected.is_none());
  assert!(state.take_focus_request().is_none());
  state.toggle(&items);
  assert!(state.select(0, 1));
  state.refresh(&[]);
  assert!(state.selected.is_none());
  assert!(state.take_focus_request().is_none());
}
