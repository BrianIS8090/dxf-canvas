use dxf::{
  Drawing, Point, Vector,
  entities::{Circle, Entity, EntityType, Insert, Line, LwPolyline},
  enums::{AcadVersion, Units},
};

use crate::{
  diagnostics::{self, DiagnosticReport, IssueKind},
  elevation,
  geometry::DrawingItem,
};

fn drawing() -> Drawing {
  let mut drawing = Drawing::new();
  drawing.header.version = AcadVersion::R2000;
  drawing.header.default_drawing_units = Units::Millimeters;
  drawing
}

fn imported(drawing: &Drawing) -> DrawingItem {
  let directory = tempfile::tempdir().unwrap();
  let path = directory.path().join("z-levels.dxf");
  drawing.save_file(&path).unwrap();
  let before = std::fs::read(&path).unwrap();
  let item = crate::dxf_import::load_dxf(&path).unwrap();
  assert_eq!(before, std::fs::read(path).unwrap());
  item
}

fn circle(z: f64) -> Entity {
  Entity::new(EntityType::Circle(Circle {
    center: Point::new(0.0, 0.0, z),
    radius: 5.0,
    ..Default::default()
  }))
}

fn levels(heights: &[f64]) -> DrawingItem {
  let mut drawing = drawing();
  for (index, height) in heights.iter().enumerate() {
    let mut entity = circle(*height);
    if let EntityType::Circle(circle) = &mut entity.specific {
      circle.center.x = index as f64 * 20.0;
    }
    drawing.add_entity(entity);
  }
  imported(&drawing)
}

fn check(item: &DrawingItem) -> DiagnosticReport {
  let mut report = DiagnosticReport::default();
  elevation::analyze(item, &mut report);
  report
}

#[test]
fn one_nonzero_level_is_valid_and_source_transform_does_not_change_it() {
  let mut item = levels(&[42.0, 42.0, 42.0]);
  item.scale = 5.0;
  item.offset = crate::geometry::Point::new(300.0, -50.0);
  item.rotation = crate::geometry::Rotation::new(1.0);
  let report = check(&item);
  assert!(report.findings.is_empty());
  assert_eq!(report.elevation.checked, 3);
  assert_eq!(report.elevation.levels[0].range.min, 42.0);
  assert!(report.elevation.summary().contains("в порядке"));
}

#[test]
fn minority_lists_layer_identity_and_signed_deviation_and_can_focus() {
  let item = levels(&[10.0, 10.0, 10.0, 15.0, -2.0]);
  let report = diagnostics::analyze(&item);
  assert_eq!(report.count(IssueKind::ZLevel), 2);
  let rows: Vec<_> = report
    .findings
    .iter()
    .filter(|f| f.kind == IssueKind::ZLevel)
    .collect();
  assert!(rows[0].detail.contains("ΔZ +5 мм"));
  assert!(rows[1].detail.contains("ΔZ -12 мм"));
  assert!(rows[0].detail.contains("CIRCLE #"));
  assert!(rows[0].detail.contains("слой «0»"));
  let bounds = rows[0].marker.bounds(&item).unwrap();
  assert!((bounds.center().x - 60.0).abs() < 1e-9);
  assert!(rows[0].marker.focus_bounds(&item).unwrap().is_valid());
}

#[test]
fn tied_or_plurality_levels_do_not_claim_a_majority() {
  for heights in [
    &[0.0, 0.0, 1.0, 1.0][..],
    &[0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0][..],
  ] {
    let report = check(&levels(heights));
    assert!(report.elevation.majority.is_none());
    assert_eq!(report.count(IssueKind::ZLevel), heights.len());
    assert!(report.elevation.summary().contains("большинства нет"));
  }
}

#[test]
fn tolerance_does_not_chain_different_levels_together() {
  let report = check(&levels(&[2.0, 2.004, 2.009]));
  assert!(report.findings.is_empty());
  let report = check(&levels(&[0.0, 0.009, 0.018]));
  assert_eq!(report.elevation.levels.len(), 2);
  assert_eq!(report.count(IssueKind::ZLevel), 1);
  let report = check(&levels(&[-0.009, 0.0, 0.009, 0.009]));
  let main = report.elevation.majority.unwrap();
  assert_eq!(report.elevation.levels[main].count, 3);
  assert_eq!(report.count(IssueKind::ZLevel), 1);
  assert!(report.findings[0].detail.contains("Объект 1:"));
}

#[test]
fn units_are_converted_before_comparing_and_unknown_units_are_explicit() {
  let mut drawing = drawing();
  drawing.header.default_drawing_units = Units::Meters;
  for z in [1.0, 1.000005, 1.002] {
    drawing.add_entity(circle(z));
  }
  let report = check(&imported(&drawing));
  assert_eq!(report.count(IssueKind::ZLevel), 1);
  assert_eq!(report.elevation.levels.len(), 2);
  assert!(report.findings[0].detail.contains("1002 мм"));
  drawing.header.default_drawing_units = Units::Unitless;
  let report = check(&imported(&drawing));
  assert!(report.elevation.summary().contains("ед. DXF"));
}

#[test]
fn hidden_objects_and_parent_layers_do_not_change_majority() {
  let mut drawing = drawing();
  drawing.add_entity(circle(7.0));
  drawing.add_entity(circle(7.0));
  drawing.add_block(dxf::Block {
    name: "Hidden block".into(),
    entities: vec![circle(55.0)],
    ..Default::default()
  });
  let mut insert = Entity::new(EntityType::Insert(Insert {
    name: "Hidden block".into(),
    ..Default::default()
  }));
  insert.common.layer = "Скрытый".into();
  drawing.add_entity(insert);
  let mut hidden = circle(-10.0);
  hidden.common.is_visible = false;
  drawing.add_entity(hidden);
  let mut item = imported(&drawing);
  item
    .appearance
    .layers
    .iter_mut()
    .find(|layer| layer.name == "Скрытый")
    .unwrap()
    .visible = false;
  let report = check(&item);
  assert_eq!(report.elevation.checked, 2);
  assert!(report.findings.is_empty());
  item.appearance.reset_layers();
  assert_eq!(check(&item).count(IssueKind::ZLevel), 1);
}

#[test]
fn nested_inserts_apply_base_z_scale_and_ocs_and_count_each_instance() {
  let mut drawing = drawing();
  drawing.add_block(dxf::Block {
    name: "Inner".into(),
    base_point: Point::new(0.0, 0.0, 2.0),
    entities: vec![circle(5.0)],
    ..Default::default()
  });
  drawing.add_block(dxf::Block {
    name: "Outer".into(),
    base_point: Point::new(0.0, 0.0, 1.0),
    entities: vec![Entity::new(EntityType::Insert(Insert {
      name: "Inner".into(),
      location: Point::new(0.0, 0.0, 10.0),
      z_scale_factor: 2.0,
      ..Default::default()
    }))],
    ..Default::default()
  });
  drawing.add_entity(Entity::new(EntityType::Insert(Insert {
    name: "Outer".into(),
    location: Point::new(0.0, 0.0, 100.0),
    z_scale_factor: -3.0,
    row_count: 2,
    row_spacing: 20.0,
    ..Default::default()
  })));
  drawing.add_entity(circle(55.0));
  let item = imported(&drawing);
  let report = check(&item);
  assert_eq!(report.elevation.checked, 3);
  assert!(report.findings.is_empty());
  assert_eq!(report.elevation.levels[0].range.min, 55.0);
  assert!(item.appearance.elevations[0].source.contains("/Outer#"));
  assert!(item.appearance.elevations[0].source.contains("/Inner#"));
}

#[test]
fn negative_normal_and_lwpolyline_elevation_are_world_coordinates() {
  let mut drawing = drawing();
  drawing.add_entity(circle(-8.0));
  let mut flipped = circle(8.0);
  if let EntityType::Circle(circle) = &mut flipped.specific {
    circle.normal = Vector::new(0.0, 0.0, -1.0);
  }
  drawing.add_entity(flipped);
  let mut poly = Entity::new(EntityType::LwPolyline(LwPolyline {
    vertices: vec![
      dxf::LwPolylineVertex {
        x: 0.0,
        y: 0.0,
        ..Default::default()
      },
      dxf::LwPolylineVertex {
        x: 10.0,
        y: 0.0,
        ..Default::default()
      },
    ],
    ..Default::default()
  }));
  poly.common.elevation = -8.0;
  drawing.add_entity(poly);
  let report = check(&imported(&drawing));
  assert!(report.findings.is_empty());
  assert_eq!(report.elevation.checked, 3);
  assert_eq!(report.elevation.levels[0].range.min, -8.0);
}

#[test]
fn inclined_lines_and_circles_report_ranges_not_just_centers() {
  let mut drawing = drawing();
  for _ in 0..3 {
    drawing.add_entity(circle(0.0));
  }
  drawing.add_entity(Entity::new(EntityType::Line(Line::new(
    Point::new(0.0, 0.0, 0.0),
    Point::new(10.0, 0.0, 3.0),
  ))));
  let mut tilted = circle(0.0);
  if let EntityType::Circle(circle) = &mut tilted.specific {
    circle.normal = Vector::new(0.0, 1.0, 0.0);
  }
  drawing.add_entity(tilted);
  let report = check(&imported(&drawing));
  assert_eq!(report.elevation.varying, 2);
  assert_eq!(report.count(IssueKind::ZLevel), 2);
  assert!(report.findings[1].detail.contains("-5…5 мм"));
}

#[test]
fn tilted_insert_propagates_xy_into_z_and_rotation_is_respected() {
  let mut drawing = drawing();
  drawing.add_block(dxf::Block {
    name: "Tilt".into(),
    entities: vec![Entity::new(EntityType::Line(Line::new(
      Point::new(0.0, 0.0, 0.0),
      Point::new(10.0, 0.0, 0.0),
    )))],
    ..Default::default()
  });
  drawing.add_entity(Entity::new(EntityType::Insert(Insert {
    name: "Tilt".into(),
    rotation: 90.0,
    extrusion_direction: Vector::new(0.0, 1.0, 0.0),
    ..Default::default()
  })));
  let item = imported(&drawing);
  let range = item.appearance.elevations[0].range.unwrap();
  assert!((range.min - 0.0).abs() < 1e-9);
  assert!((range.max - 10.0).abs() < 1e-9);
  assert_eq!(check(&item).elevation.varying, 1);
}

#[test]
fn spline_height_range_is_preserved_and_marked_as_estimated() {
  let mut drawing = drawing();
  drawing.add_entity(Entity::new(EntityType::Spline(dxf::entities::Spline {
    degree_of_curve: 2,
    control_points: vec![
      Point::new(0.0, 0.0, 0.0),
      Point::new(10.0, 5.0, 4.0),
      Point::new(20.0, 0.0, 0.0),
    ],
    knot_values: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
    ..Default::default()
  })));
  let report = check(&imported(&drawing));
  assert_eq!(report.elevation.varying, 1);
  assert!(report.findings[0].detail.contains("оценочный"));
  assert!(report.findings[0].detail.contains("0…4 мм"));
}

#[test]
fn invalid_z_does_not_become_zero_or_a_successful_check() {
  let mut item = levels(&[0.0, 0.0, 1.0]);
  item.appearance.elevations[2].range = elevation::ZRange::values([f64::NAN]);
  let report = check(&item);
  assert_eq!(report.elevation.unknown, 1);
  assert_eq!(report.count(IssueKind::ZLevel), 1);
  assert!(!report.elevation.summary().contains("в порядке"));
}

#[test]
fn large_level_list_uses_sorted_groups_not_pairwise_comparison() {
  let mut item = levels(&[0.0]);
  let source = item.appearance.elevations[0].clone();
  item.appearance.elevations = (0..50_000)
    .map(|i| {
      let mut object = source.clone();
      object.range = elevation::ZRange::values([if i < 49_999 { 20.0 } else { 21.0 }]);
      object
    })
    .collect();
  let report = check(&item);
  assert_eq!(report.elevation.levels.len(), 2);
  assert_eq!(report.count(IssueKind::ZLevel), 1);
}

#[test]
fn classic_2d_and_3d_polyline_heights_are_not_confused() {
  let mut drawing = drawing();
  for (flags, header_z, vertices) in [
    (0, 12.0, [0.0, 0.0]),
    (8, 999.0, [2.0, 6.0]),
    (16, 999.0, [3.0, 7.0]),
  ] {
    let mut poly = dxf::entities::Polyline {
      flags,
      location: Point::new(0.0, 0.0, header_z),
      ..Default::default()
    };
    for (i, z) in vertices.into_iter().enumerate() {
      poly.add_vertex(
        &mut drawing,
        dxf::entities::Vertex {
          location: Point::new(i as f64 * 20.0, 0.0, z),
          flags: if flags == 16 { 64 } else { 0 },
          ..Default::default()
        },
      );
    }
    drawing.add_entity(Entity::new(EntityType::Polyline(poly)));
  }
  let item = imported(&drawing);
  let ranges: Vec<_> = item
    .appearance
    .elevations
    .iter()
    .map(|o| {
      let r = o
        .range
        .unwrap_or_else(|| panic!("Не прочитана высота {}", o.source));
      (r.min, r.max)
    })
    .collect();
  assert_eq!(ranges, [(12.0, 12.0), (2.0, 6.0), (3.0, 7.0)]);
  // Писатель библиотеки сбрасывает бит 64 вершин polyface; проверяем чтение
  // корректных флагов 192 напрямую, отдельно от этой особенности сериализации.
  let source = "0\nSECTION\n2\nENTITIES\n0\nPOLYLINE\n70\n64\n0\nVERTEX\n70\n192\n10\n0\n20\n0\n30\n3\n0\nVERTEX\n70\n192\n10\n10\n20\n0\n30\n7\n0\nVERTEX\n70\n128\n10\n0\n20\n0\n30\n0\n71\n1\n72\n2\n73\n1\n0\nSEQEND\n0\nENDSEC\n0\nEOF\n";
  let parsed = Drawing::load(&mut source.as_bytes()).unwrap();
  let range = elevation::entity_range(
    parsed.entities().next().unwrap(),
    elevation::Transform3::IDENTITY,
  )
  .unwrap();
  assert_eq!((range.min, range.max), (3.0, 7.0));
}

#[test]
fn tilted_arc_bulge_and_ellipse_include_interior_z_extrema() {
  let mut drawing = drawing();
  drawing.add_entity(Entity::new(EntityType::Arc(dxf::entities::Arc {
    radius: 10.0,
    normal: Vector::new(0.0, 1.0, 0.0),
    start_angle: 20.0,
    end_angle: 160.0,
    ..Default::default()
  })));
  drawing.add_entity(Entity::new(EntityType::LwPolyline(LwPolyline {
    vertices: vec![
      dxf::LwPolylineVertex {
        x: -10.0,
        bulge: 1.0,
        ..Default::default()
      },
      dxf::LwPolylineVertex {
        x: 10.0,
        ..Default::default()
      },
    ],
    extrusion_direction: Vector::new(0.0, 1.0, 0.0),
    ..Default::default()
  })));
  drawing.add_entity(Entity::new(EntityType::Ellipse(dxf::entities::Ellipse {
    center: Point::new(0.0, 0.0, 4.0),
    major_axis: Vector::new(10.0, 0.0, 0.0),
    normal: Vector::new(0.0, 1.0, 0.0),
    minor_axis_ratio: 0.5,
    ..Default::default()
  })));
  let item = imported(&drawing);
  let ranges: Vec<_> = item
    .appearance
    .elevations
    .iter()
    .map(|o| o.range.unwrap())
    .collect();
  assert!((ranges[0].max - 10.0).abs() < 1e-9);
  assert!((ranges[1].min + 10.0).abs() < 1e-9);
  assert!((ranges[2].min + 1.0).abs() < 1e-9);
  assert!((ranges[2].max - 9.0).abs() < 1e-9);
}

#[test]
fn hatch_is_one_object_and_dimension_graphics_are_excluded() {
  let source = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nHATCH\n5\nAB\n8\n0\n30\n15\n70\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n4\n10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n10\n10\n0\n20\n10\n97\n0\n75\n0\n0\nENDSEC\n0\nEOF\n";
  let directory = tempfile::tempdir().unwrap();
  let path = directory.path().join("hatch.dxf");
  std::fs::write(&path, source).unwrap();
  let item = crate::dxf_import::load_dxf(&path).unwrap();
  assert_eq!(item.appearance.elevations.len(), 1);
  assert_eq!(item.appearance.elevations[0].range.unwrap().min, 15.0);
  assert!(item.appearance.elevations[0].source.contains("HATCH #AB"));
  let mut drawing = drawing();
  drawing.add_entity(circle(20.0));
  drawing.add_block(dxf::Block {
    name: "*D1".into(),
    entities: vec![circle(500.0)],
    ..Default::default()
  });
  drawing.add_entity(Entity::new(EntityType::RotatedDimension(
    dxf::entities::RotatedDimension {
      dimension_base: dxf::entities::DimensionBase {
        block_name: "*D1".into(),
        ..Default::default()
      },
      ..Default::default()
    },
  )));
  let report = check(&imported(&drawing));
  assert_eq!(report.elevation.checked, 1);
  assert!(report.findings.is_empty());
}

#[test]
fn annotations_and_fills_are_locatable_and_incomplete_import_is_not_all_clear() {
  let mut drawing = drawing();
  drawing.add_entity(circle(0.0));
  drawing.add_entity(circle(0.0));
  drawing.add_entity(Entity::new(EntityType::Text(dxf::entities::Text {
    value: "Высота".into(),
    location: Point::new(20.0, 0.0, 5.0),
    text_height: 2.0,
    ..Default::default()
  })));
  let item = imported(&drawing);
  let report = check(&item);
  assert_eq!(report.count(IssueKind::ZLevel), 1);
  assert!(matches!(
    report.findings[0].marker,
    crate::diagnostics::Marker::Bounds(_)
  ));
  assert!(report.findings[0].marker.focus_bounds(&item).is_some());
  let mut item = levels(&[4.0, 4.0]);
  item.unsupported_entities = 1;
  assert!(!check(&item).elevation.summary().contains("в порядке"));
  assert!(check(&item).elevation.summary().contains("неполная"));
}

#[test]
fn public_demo_has_two_locatable_height_outliers() {
  let item = crate::dxf_import::load_dxf(
    &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/elevation_demo.dxf"),
  )
  .unwrap();
  let report = diagnostics::analyze(&item);
  assert_eq!(report.findings.len(), 2);
  assert_eq!(report.elevation.checked, 6);
  assert_eq!(
    report.elevation.levels[report.elevation.majority.unwrap()].count,
    4
  );
  for finding in report.findings {
    assert!(finding.marker.focus_bounds(&item).unwrap().is_valid());
  }
}

#[test]
fn aligned_text_uses_its_alignment_point_and_invalid_spline_weights_are_unknown() {
  let source = "0\nSECTION\n2\nENTITIES\n0\nTEXT\n10\n0\n20\n0\n30\n0\n11\n10\n21\n10\n31\n25\n40\n2\n1\nTEST\n72\n1\n0\nENDSEC\n0\nEOF\n";
  let parsed = Drawing::load(&mut source.as_bytes()).unwrap();
  let range = elevation::entity_range(
    parsed.entities().next().unwrap(),
    elevation::Transform3::IDENTITY,
  )
  .unwrap();
  assert_eq!((range.min, range.max), (25.0, 25.0));
  let spline = Entity::new(EntityType::Spline(dxf::entities::Spline {
    control_points: vec![Point::new(0.0, 0.0, 0.0), Point::new(10.0, 0.0, 0.0)],
    weight_values: vec![1.0, -1.0],
    ..Default::default()
  }));
  assert!(elevation::entity_range(&spline, elevation::Transform3::IDENTITY).is_none());
}
