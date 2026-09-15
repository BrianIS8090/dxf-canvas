use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, FontId, Stroke};

#[derive(Clone, Copy)]
pub struct Palette {
  pub dark: bool,
  pub canvas: Color32,
  pub panel: Color32,
  pub card: Color32,
  pub selected: Color32,
  pub text: Color32,
  pub muted: Color32,
  pub border: Color32,
  pub accent: Color32,
  pub warning: Color32,
  pub success: Color32,
  pub dimension: Color32,
}

impl Palette {
  pub fn get(context: &egui::Context) -> Self {
    Self::new(context.theme() == egui::Theme::Dark)
  }

  pub fn new(dark: bool) -> Self {
    let rgb = Color32::from_rgb;
    if dark {
      Self {
        dark,
        canvas: rgb(24, 29, 37),
        panel: rgb(33, 40, 50),
        card: rgb(40, 49, 61),
        selected: rgb(40, 63, 89),
        text: rgb(231, 236, 244),
        muted: rgb(171, 184, 201),
        border: rgb(68, 82, 101),
        accent: rgb(123, 182, 250),
        warning: rgb(248, 188, 105),
        success: rgb(92, 212, 170),
        dimension: rgb(255, 184, 119),
      }
    } else {
      Self {
        dark,
        canvas: rgb(239, 241, 244),
        panel: Color32::WHITE,
        card: rgb(248, 250, 252),
        selected: rgb(238, 245, 253),
        text: rgb(37, 48, 63),
        muted: rgb(92, 105, 121),
        border: rgb(218, 224, 231),
        accent: rgb(30, 94, 168),
        warning: rgb(155, 88, 13),
        success: rgb(27, 121, 83),
        dimension: rgb(164, 73, 22),
      }
    }
  }

  pub fn drawing_color(self, color: Color32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    if !self.dark {
      if r > 235 && g > 235 && b > 235 {
        return Color32::from_rgba_unmultiplied(31, 37, 46, a);
      }
      return color;
    }
    // Осветляем только плохо различимые цвета, сохраняя оттенок и прозрачность.
    // Данные чертежа не меняются; преобразование выполняется при отрисовке.
    let brightness = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    if brightness < 110.0 {
      let mix = (150.0 - brightness) / (255.0 - brightness);
      let lift = |v: u8| (v as f32 + (255.0 - v as f32) * mix).round() as u8;
      Color32::from_rgba_unmultiplied(lift(r), lift(g), lift(b), a)
    } else {
      color
    }
  }
}

pub fn configure(context: &egui::Context) {
  for theme in [egui::Theme::Light, egui::Theme::Dark] {
    let palette = Palette::new(theme == egui::Theme::Dark);
    let mut style = (*context.style_of(theme)).clone();
    for (kind, size) in [
      (egui::TextStyle::Body, 14.0),
      (egui::TextStyle::Button, 14.0),
      (egui::TextStyle::Small, 12.0),
      (egui::TextStyle::Heading, 18.0),
    ] {
      style.text_styles.insert(kind, FontId::proportional(size));
    }
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.visuals = if palette.dark {
      egui::Visuals::dark()
    } else {
      egui::Visuals::light()
    };
    style.visuals.override_text_color = Some(palette.text);
    style.visuals.panel_fill = palette.panel;
    style.visuals.window_fill = palette.panel;
    style.visuals.extreme_bg_color = if palette.dark {
      palette.canvas
    } else {
      Color32::WHITE
    };
    style.visuals.widgets.inactive.weak_bg_fill = palette.card;
    style.visuals.widgets.inactive.bg_fill = palette.card;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    style.visuals.selection.bg_fill = palette.selected;
    style.visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    style.visuals.widgets.active.bg_fill = palette.selected;
    style.visuals.widgets.active.weak_bg_fill = palette.selected;
    style.visuals.widgets.hovered.bg_fill = palette.selected;
    style.visuals.widgets.hovered.weak_bg_fill = palette.selected;
    style.visuals.hyperlink_color = palette.accent;
    style.visuals.warn_fg_color = palette.warning;
    style.visuals.error_fg_color = if palette.dark {
      Color32::from_rgb(255, 131, 131)
    } else {
      Color32::from_rgb(175, 45, 45)
    };
    context.set_style_of(theme, style);
  }
  apply(context, false);
}

pub fn apply(context: &egui::Context, dark: bool) {
  context.set_theme(if dark {
    egui::ThemePreference::Dark
  } else {
    egui::ThemePreference::Light
  });
  context.send_viewport_cmd(egui::ViewportCommand::SetTheme(if dark {
    egui::SystemTheme::Dark
  } else {
    egui::SystemTheme::Light
  }));
  context.request_repaint();
}

fn preference_path() -> Option<PathBuf> {
  std::env::var_os("LOCALAPPDATA")
    .filter(|value| !value.is_empty())
    .map(|path| PathBuf::from(path).join("DXF-Canvas").join("theme.txt"))
}

fn read_preference(path: &Path) -> bool {
  std::fs::read_to_string(path).is_ok_and(|text| text.trim() == "dark")
}

fn write_preference(path: &Path, dark: bool) -> std::io::Result<()> {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  std::fs::write(path, if dark { "dark\n" } else { "light\n" })
}

pub fn load_preference() -> bool {
  preference_path().is_some_and(|path| read_preference(&path))
}

pub fn save_preference(dark: bool) -> std::io::Result<()> {
  if let Some(path) = preference_path() {
    write_preference(&path, dark)
  } else {
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn theme_text_and_measurements_have_sufficient_contrast() {
    let luminance = |color: Color32| {
      let channel = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.04045 {
          v / 12.92
        } else {
          ((v + 0.055) / 1.055).powf(2.4)
        }
      };
      0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    };
    for dark in [false, true] {
      let palette = Palette::new(dark);
      for fg in [
        palette.text,
        palette.muted,
        palette.dimension,
        palette.accent,
      ] {
        for bg in [palette.panel, palette.canvas] {
          let (a, b) = (luminance(fg), luminance(bg));
          assert!(
            (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5,
            "Недостаточный контраст: {fg:?} / {bg:?}"
          );
        }
      }
    }
  }

  #[test]
  fn preference_round_trip_and_missing_or_invalid_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings/theme.txt");
    assert!(!read_preference(&path));
    write_preference(&path, true).unwrap();
    assert!(read_preference(&path));
    write_preference(&path, false).unwrap();
    assert!(!read_preference(&path));
    std::fs::write(&path, "invalid").unwrap();
    assert!(!read_preference(&path));
  }

  #[test]
  fn cad_colors_keep_alpha_and_remain_readable_in_both_themes() {
    let dark = Palette::new(true);
    let light = Palette::new(false);
    for original in [
      Color32::BLACK,
      Color32::WHITE,
      Color32::BLUE,
      Color32::from_rgb(31, 37, 46),
      Color32::from_rgba_unmultiplied(10, 20, 30, 128),
    ] {
      let result = dark.drawing_color(original);
      assert_eq!(result.a(), original.a());
      let [r, g, b, _] = result.to_srgba_unmultiplied();
      assert!(0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32 >= 110.0);
    }
    assert_eq!(dark.drawing_color(Color32::YELLOW), Color32::YELLOW);
    assert_eq!(light.drawing_color(Color32::RED), Color32::RED);
    assert_ne!(light.drawing_color(Color32::WHITE), Color32::WHITE);
  }
}
