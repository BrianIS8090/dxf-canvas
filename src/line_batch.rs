use eframe::egui::{
  self,
  epaint::{Mesh, Shape, tessellator::Path},
};

/// Объединяет соседние прямые; кривые и тексты сохраняют свой порядок отрисовки.
pub struct LineBatch<'a> {
  painter: &'a egui::Painter,
  path: Path,
  mesh: Mesh,
  feathering: f32,
  shapes: Vec<Shape>,
}

impl<'a> LineBatch<'a> {
  pub fn new(painter: &'a egui::Painter) -> Self {
    Self {
      painter,
      path: Path::default(),
      mesh: Mesh::default(),
      feathering: 1.0 / painter.ctx().pixels_per_point(),
      shapes: Vec::new(),
    }
  }

  pub fn line(&mut self, points: [egui::Pos2; 2], stroke: egui::Stroke) {
    self.path.clear();
    self.path.add_line_segment(points);
    self
      .path
      .stroke_open(self.feathering, &stroke.into(), &mut self.mesh);
    if self.mesh.vertices.len() >= 32_768 {
      self.flush();
    }
  }

  pub fn flush(&mut self) {
    if !self.mesh.is_empty() {
      self
        .shapes
        .push(Shape::mesh(std::mem::take(&mut self.mesh)));
    }
  }

  pub fn push(&mut self, shape: Shape) {
    self.flush();
    self.shapes.push(shape);
  }

  pub fn clip_rect(&self) -> egui::Rect {
    self.painter.clip_rect()
  }

  pub fn finish(&mut self) {
    self.flush();
    // Один доступ к списку интерфейса вместо десятков тысяч отдельных блокировок.
    self.painter.extend(std::mem::take(&mut self.shapes));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn grouped_lines_match_regular_strokes_including_color_width_and_dpi() {
    for dpi in [1.0, 2.0] {
      let render = |grouped: bool| {
        let context = egui::Context::default();
        context.set_pixels_per_point(dpi);
        let mut output = context.run_ui(Default::default(), |ui| {
          let mut batch = LineBatch::new(ui.painter());
          for (i, width) in [0.7, 1.0, 2.5].into_iter().enumerate() {
            let points = [
              egui::pos2(10.25, 10.5 + i as f32 * 20.0),
              egui::pos2(180.75, 17.25 + i as f32 * 20.0),
            ];
            let stroke = egui::Stroke::new(
              width,
              egui::Color32::from_rgba_unmultiplied(30, 160, 220, 170),
            );
            if grouped {
              batch.line(points, stroke);
            } else {
              ui.painter().add(Shape::line(points.to_vec(), stroke));
            }
          }
          batch.finish();
        });
        output.textures_delta.clear();
        let mut mesh = Mesh::default();
        for primitive in context.tessellate(output.shapes, output.pixels_per_point) {
          if let egui::epaint::Primitive::Mesh(part) = primitive.primitive {
            mesh.append(part);
          }
        }
        mesh
      };
      let regular = render(false);
      let grouped = render(true);
      assert_eq!(regular.vertices, grouped.vertices);
      assert_eq!(regular.indices, grouped.indices);
    }
  }
}
