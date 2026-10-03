use eframe::egui::{self, ColorImage, TextureHandle, Vec2};

#[derive(Clone, Copy, Debug)]
pub enum Icon {
  Open,
  Fit,
  Arrange,
  Settings,
  Help,
  Sun,
  Moon,
  Select,
  Linear,
  Diameter,
  Radius,
  Angle,
  Region,
  Undo,
  Dimensions,
  Check,
  Sidebar,
  Files,
  Layers,
  Search,
  Close,
  Focus,
  Minus,
  Plus,
  Download,
  Trash,
}

impl Icon {
  fn bytes(self) -> &'static [u8] {
    match self {
      Self::Open => include_bytes!("../assets/lucide/folder-open.png"),
      Self::Fit => include_bytes!("../assets/lucide/scan.png"),
      Self::Arrange => include_bytes!("../assets/lucide/layout-grid.png"),
      Self::Settings => include_bytes!("../assets/lucide/sliders-horizontal.png"),
      Self::Help => include_bytes!("../assets/lucide/circle-question-mark.png"),
      Self::Sun => include_bytes!("../assets/lucide/sun.png"),
      Self::Moon => include_bytes!("../assets/lucide/moon.png"),
      Self::Select => include_bytes!("../assets/lucide/mouse-pointer-2.png"),
      Self::Linear => include_bytes!("../assets/lucide/ruler.png"),
      Self::Diameter => include_bytes!("../assets/lucide/diameter.png"),
      Self::Radius => include_bytes!("../assets/lucide/radius.png"),
      Self::Angle => include_bytes!("../assets/lucide/triangle.png"),
      Self::Region => include_bytes!("../assets/lucide/pentagon.png"),
      Self::Undo => include_bytes!("../assets/lucide/undo-2.png"),
      Self::Dimensions => include_bytes!("../assets/lucide/list.png"),
      Self::Check => include_bytes!("../assets/lucide/scan-line.png"),
      Self::Sidebar => include_bytes!("../assets/lucide/panel-right.png"),
      Self::Files => include_bytes!("../assets/lucide/files.png"),
      Self::Layers => include_bytes!("../assets/lucide/layers.png"),
      Self::Search => include_bytes!("../assets/lucide/search.png"),
      Self::Close => include_bytes!("../assets/lucide/x.png"),
      Self::Focus => include_bytes!("../assets/lucide/focus.png"),
      Self::Minus => include_bytes!("../assets/lucide/minus.png"),
      Self::Plus => include_bytes!("../assets/lucide/plus.png"),
      Self::Download => include_bytes!("../assets/lucide/download.png"),
      Self::Trash => include_bytes!("../assets/lucide/trash.png"),
    }
  }

  pub fn image(self, context: &egui::Context, size: f32) -> egui::Image<'static> {
    let id = egui::Id::new(format!("lucide-{self:?}"));
    // Текстура создаётся один раз на контекст, а цвет задаёт тема кнопки.
    let texture = context
      .data_mut(|data| data.get_temp::<TextureHandle>(id))
      .unwrap_or_else(|| {
        let image = image::load_from_memory(self.bytes())
          .expect("Встроенная иконка Lucide")
          .to_rgba8();
        let texture = context.load_texture(
          format!("{self:?}"),
          ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
          ),
          egui::TextureOptions::LINEAR,
        );
        context.data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
      });
    egui::Image::from_texture((texture.id(), Vec2::splat(size)))
  }
}

pub fn button(
  ui: &mut egui::Ui,
  icon: Icon,
  label: &str,
  text: bool,
  enabled: bool,
  selected: bool,
) -> egui::Response {
  let image = icon.image(ui.ctx(), 18.0);
  let button = if text {
    egui::Button::new((image, label))
  } else {
    egui::Button::new(image)
  }
  .image_tint_follows_text_color(true)
  .min_size(Vec2::splat(32.0))
  .corner_radius(6.0)
  .selected(selected)
  .frame_when_inactive(selected);
  let response = ui.add_enabled(enabled, button);
  response.widget_info(|| {
    egui::WidgetInfo::selected(
      egui::WidgetType::Button,
      enabled && ui.is_enabled(),
      selected,
      label,
    )
  });
  #[cfg(test)]
  ui.ctx()
    .data_mut(|data| data.insert_temp(egui::Id::new(("control", label)), response.rect));
  response.on_hover_text(label)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn all_icons_are_embedded_white_masks_for_both_themes() {
    for icon in [
      Icon::Open,
      Icon::Fit,
      Icon::Arrange,
      Icon::Settings,
      Icon::Help,
      Icon::Sun,
      Icon::Moon,
      Icon::Select,
      Icon::Linear,
      Icon::Diameter,
      Icon::Radius,
      Icon::Angle,
      Icon::Region,
      Icon::Undo,
      Icon::Dimensions,
      Icon::Check,
      Icon::Sidebar,
      Icon::Files,
      Icon::Layers,
      Icon::Search,
      Icon::Close,
      Icon::Focus,
      Icon::Minus,
      Icon::Plus,
      Icon::Download,
      Icon::Trash,
    ] {
      let image = image::load_from_memory(icon.bytes()).unwrap().to_rgba8();
      assert_eq!(image.dimensions(), (96, 96));
      assert!(image.pixels().any(|p| p[3] > 200));
      for pixel in image.pixels().filter(|p| p[3] > 0) {
        assert_eq!(&pixel.0[..3], &[255, 255, 255]);
      }
    }
  }
}
