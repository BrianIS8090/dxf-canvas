use std::sync::Arc;

use eframe::egui::{FontData, FontDefinitions, FontFamily};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CadFont {
  #[default]
  Sans,
  OpenSans,
  CondensedLight,
}

impl CadFont {
  pub fn from_name(name: &str) -> Option<Self> {
    let name = name
      .rsplit(['/', '\\'])
      .next()
      .unwrap_or(name)
      .to_ascii_lowercase();
    let name = name.trim_end_matches(".ttf").replace([' ', '-', '_'], "");
    match name.as_str() {
      "opensanscondensedlight" => Some(Self::CondensedLight),
      "opensans" | "opensansregular" => Some(Self::OpenSans),
      "arial" | "arialregular" => Some(Self::Sans),
      _ => None,
    }
  }

  pub fn family(self) -> FontFamily {
    FontFamily::Name(
      match self {
        Self::Sans => "cad-sans",
        Self::OpenSans => "cad-open-sans",
        Self::CondensedLight => "cad-condensed-light",
      }
      .into(),
    )
  }
}

pub fn install(fonts: &mut FontDefinitions) {
  fonts.font_data.insert(
    "cad-open-sans".into(),
    Arc::new(FontData::from_static(include_bytes!(
      "../assets/fonts/OpenSans-Regular.ttf"
    ))),
  );
  fonts.font_data.insert(
    "cad-condensed-light".into(),
    Arc::new(FontData::from_static(include_bytes!(
      "../assets/fonts/OpenSans-CondensedLight.ttf"
    ))),
  );
  // Пути из DXF не открываем: используем только встроенные и известный системный шрифт.
  let default_font = if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\arial.ttf") {
    fonts
      .font_data
      .insert("cad-arial".into(), Arc::new(FontData::from_owned(bytes)));
    "cad-arial"
  } else {
    "cad-open-sans"
  };
  let fallback = fonts
    .families
    .get(&FontFamily::Proportional)
    .cloned()
    .unwrap_or_default();
  for (font, name) in [
    (CadFont::Sans, default_font),
    (CadFont::OpenSans, "cad-open-sans"),
    (CadFont::CondensedLight, "cad-condensed-light"),
  ] {
    let mut names = vec![name.to_owned()];
    names.extend(fallback.iter().cloned());
    fonts.families.insert(font.family(), names);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn font_aliases_preserve_condensed_style_without_using_arbitrary_paths() {
    for name in [
      "OpenSansCondensed-Light",
      "OpenSans-CondensedLight.ttf",
      "Open Sans Condensed Light",
    ] {
      assert_eq!(CadFont::from_name(name), Some(CadFont::CondensedLight));
    }
    assert_eq!(CadFont::from_name("custom.shx"), None);
    let mut fonts = FontDefinitions::default();
    let ui_fonts = fonts.families.clone();
    install(&mut fonts);
    for (family, names) in ui_fonts {
      assert_eq!(fonts.families[&family], names);
    }
  }
}
