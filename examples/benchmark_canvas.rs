#![allow(dead_code)]
#[path = "../src/cad_render.rs"]
mod cad_render;
#[path = "../src/cad_scene.rs"]
mod cad_scene;
#[path = "../src/cad_text.rs"]
mod cad_text;
#[path = "../src/display_geometry.rs"]
mod display_geometry;
#[path = "../src/dxf_import.rs"]
mod dxf_import;
#[path = "../src/dxf_scene.rs"]
mod dxf_scene;
#[path = "../src/geometry.rs"]
mod geometry;
#[path = "../src/hatch.rs"]
mod hatch;
#[path = "../src/line_batch.rs"]
mod line_batch;
#[path = "../src/measurement.rs"]
mod measurement;
#[path = "../src/planar.rs"]
mod planar;
#[path = "../src/raw_dxf.rs"]
mod raw_dxf;
#[path = "../src/region.rs"]
mod region;
#[path = "../src/spatial.rs"]
mod spatial;

use eframe::egui;
use std::time::Instant;

fn median(values: &mut [f64]) -> f64 {
  values.sort_by(f64::total_cmp);
  values[values.len() / 2]
}

fn main() {
  let path = std::env::args_os().nth(1).expect("Нужен путь к DXF");
  let mode = std::env::args().nth(2).unwrap_or_else(|| "all".into());
  let view_mode = std::env::args().nth(3).unwrap_or_else(|| "zoom".into());
  let started = Instant::now();
  let mut item = dxf_import::load_dxf(std::path::Path::new(&path)).unwrap();
  println!(
    "import_ms={:.2} primitives={} texts={} fills={}",
    started.elapsed().as_secs_f64() * 1000.0,
    item.primitives.len(),
    item.appearance.texts.len(),
    item.appearance.fills.len()
  );
  if mode == "no_text" {
    item.appearance.texts.clear();
  }
  if mode == "no_fill" {
    item.appearance.fills.clear();
  }
  if mode == "no_patterns" {
    for style in &mut item.appearance.styles {
      style.pattern = std::sync::Arc::from([]);
    }
  }
  let source_points: usize = item
    .primitives
    .iter()
    .map(|p| match p {
      geometry::Primitive::Path { points, .. } => points.len(),
      _ => 1,
    })
    .sum();
  let mut kinds = [0; 4];
  for primitive in &item.primitives {
    kinds[match primitive {
      geometry::Primitive::Point(_) => 0,
      geometry::Primitive::Path { points, .. } if points.len() == 2 => 1,
      geometry::Primitive::Path { points, .. } if points.len() <= 8 => 2,
      _ => 3,
    }] += 1;
  }
  println!("point_line_short_long={kinds:?}");
  println!(
    "source_points={source_points} patterned_paths={}",
    item
      .appearance
      .styles
      .iter()
      .filter(|style| !style.pattern.is_empty())
      .count()
  );
  if mode == "no_hatch" {
    for style in &mut item.appearance.styles {
      if !style.diagnostic {
        style.visible = false;
      }
    }
  }
  let context = egui::Context::default();
  let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2200.0, 1200.0));
  let mut center = item.bounds.center();
  let mut fit = (2100.0 / item.bounds.width()).min(1100.0 / item.bounds.height()) as f32;
  if view_mode == "detail" {
    fit *= 4.0;
    center.y = item.bounds.min.y + item.bounds.height() * 0.25;
  }
  let mut frame_times = Vec::new();
  let mut paint_times = Vec::new();
  let mut tess_times = Vec::new();
  let mut snap_times = Vec::new();
  let mut vertices = 0;
  let mut max_shapes = 0;
  for i in 0..24 {
    let scale = fit
      * if view_mode == "pan" {
        1.0
      } else {
        1.0 + (i % 12) as f32 * 0.025
      };
    let view = geometry::ViewTransform {
      scale,
      origin: egui::pos2(
        1100.0 - center.x as f32 * scale + (i % 7) as f32 * 2.0,
        600.0 + center.y as f32 * scale,
      ),
    };
    let start = Instant::now();
    let mut output = context.run_ui(
      egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
      },
      |ui| {
        cad_render::paint(&ui.painter().with_clip_rect(rect), &item, view);
      },
    );
    output.textures_delta.clear();
    let paint_ms = start.elapsed().as_secs_f64() * 1000.0;
    max_shapes = max_shapes.max(output.shapes.len());
    if i == 0 {
      println!("prepared_shapes={}", output.shapes.len());
    }
    let tess_start = Instant::now();
    let meshes = context.tessellate(output.shapes, output.pixels_per_point);
    let tess_ms = tess_start.elapsed().as_secs_f64() * 1000.0;
    let frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    vertices = meshes
      .iter()
      .map(|shape| match &shape.primitive {
        egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
        _ => 0,
      })
      .sum::<usize>();
    let snap_start = Instant::now();
    std::hint::black_box(measurement::snap_point(
      std::slice::from_ref(&item),
      view,
      egui::pos2(1100.0, 600.0),
      None,
    ));
    let snap_ms = snap_start.elapsed().as_secs_f64() * 1000.0;
    if i >= 4 {
      frame_times.push(frame_ms);
      paint_times.push(paint_ms);
      tess_times.push(tess_ms);
      snap_times.push(snap_ms);
    }
    std::hint::black_box(meshes);
  }
  let frame = median(&mut frame_times);
  let snap = median(&mut snap_times);
  let p95 = frame_times[frame_times.len() * 95 / 100];
  println!("p95_cpu_frame_ms={:.2} max_shapes={max_shapes}", p95);
  println!(
    "mode={mode} view={view_mode} median_cpu_frame_ms={frame:.2} paint_ms={:.2} tess_ms={:.2} snap_ms={snap:.2} vertices={vertices} budget_60hz_ms=16.67 verdict={}",
    median(&mut paint_times),
    median(&mut tess_times),
    if frame > 16.67 || p95 > 25.0 || snap > 2.0 {
      "SLOW"
    } else {
      "PASS"
    }
  );
  if frame > 16.67 || p95 > 25.0 || snap > 2.0 {
    std::process::exit(2);
  }
}
