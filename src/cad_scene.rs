use std::sync::Arc;

use eframe::egui::Color32;

use crate::geometry::{Bounds, Point};

#[derive(Clone, Debug)]
pub struct Layer {
  pub name: String,
  pub color: Color32,
  pub visible: bool,
  pub initial_visible: bool,
  pub locked: bool,
  pub count: usize,
}

#[derive(Clone, Debug)]
pub struct EntityStyle {
  pub layer: usize,
  pub parent_layers: Arc<[usize]>,
  pub color: Color32,
  pub visible: bool,
  pub line_weight: f32,
  pub pattern: Arc<[f64]>,
  pub diagnostic: bool,
}

impl Default for EntityStyle {
  fn default() -> Self {
    Self {
      layer: 0,
      parent_layers: Arc::from([]),
      color: Color32::from_rgb(31, 37, 46),
      visible: true,
      line_weight: 0.0,
      pattern: Arc::from([]),
      diagnostic: true,
    }
  }
}

#[derive(Clone, Debug)]
pub struct CadText {
  pub text: String,
  pub origin: Point,
  pub x_axis: Point,
  pub y_axis: Point,
  pub height: f64,
  pub width: f64,
  pub width_factor: f64,
  pub alignment: [f64; 2],
  pub line_spacing: f32,
  pub style: EntityStyle,
  pub bounds: Bounds,
}

#[derive(Clone, Debug)]
pub struct CadFill {
  pub vertices: Vec<Point>,
  pub indices: Vec<u32>,
  pub style: EntityStyle,
  pub bounds: Bounds,
}

#[derive(Clone, Debug, Default)]
pub struct Appearance {
  pub layers: Vec<Layer>,
  pub styles: Vec<EntityStyle>,
  pub primitive_bounds: Vec<Bounds>,
  pub texts: Vec<CadText>,
  pub fills: Vec<CadFill>,
  pub warnings: Vec<String>,
  pub source_counts: std::collections::BTreeMap<String, usize>,
  pub snap_index: Arc<crate::spatial::SpatialIndex>,
  pub render_index: Arc<crate::spatial::SpatialIndex>,
  pub display_geometry: Arc<crate::display_geometry::DisplayGeometry>,
}

impl Appearance {
  pub fn visible(&self, style: &EntityStyle) -> bool {
    style.visible
      && self
        .layers
        .get(style.layer)
        .is_none_or(|layer| layer.visible)
      && style
        .parent_layers
        .iter()
        .all(|index| self.layers.get(*index).is_none_or(|layer| layer.visible))
  }

  pub fn primitive_visible(&self, index: usize) -> bool {
    self
      .styles
      .get(index)
      .is_none_or(|style| self.visible(style))
  }

  pub fn primitive_diagnostic(&self, index: usize) -> bool {
    self.primitive_visible(index) && self.styles.get(index).is_none_or(|style| style.diagnostic)
  }

  pub fn reset_layers(&mut self) {
    for layer in &mut self.layers {
      layer.visible = layer.initial_visible;
    }
  }
}

pub fn indexed_color(index: u8) -> Color32 {
  let rgb = match index {
    1 => [255, 0, 0],
    2 => [255, 255, 0],
    3 => [0, 255, 0],
    4 => [0, 255, 255],
    5 => [0, 0, 255],
    6 => [255, 0, 255],
    0 | 7 | 255 => [31, 37, 46],
    8 => [128, 128, 128],
    9 => [192, 192, 192],
    250..=254 => {
      let value = [51, 80, 105, 130, 190][(index - 250) as usize];
      [value; 3]
    }
    _ => {
      let hue = ((index - 10) / 10) as f64 / 4.0;
      let shade = (index - 10) % 10;
      let value = [1.0, 0.65, 0.5, 0.3, 0.15][(shade / 2) as usize];
      let saturation = if shade.is_multiple_of(2) { 1.0 } else { 0.5 };
      let c = value * saturation;
      let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
      let m = value - c;
      let (r, g, b) = match hue as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
      };
      [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
      ]
    }
  };
  Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn readable_color(color: Color32) -> Color32 {
  // Белый цвет CAD должен оставаться видимым на светлом холсте.
  if color.r() > 235 && color.g() > 235 && color.b() > 235 {
    Color32::from_rgba_unmultiplied(31, 37, 46, color.a())
  } else {
    color
  }
}
