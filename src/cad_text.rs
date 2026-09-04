use crate::cad_scene::{indexed_color, readable_color};
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};

#[derive(Clone, Debug)]
struct Format {
  scale: f32,
  color: Color32,
  underline: bool,
  italic: bool,
}

pub fn plain(source: &str) -> String {
  layout(source, 10.0, Color32::BLACK, f32::INFINITY, 1.0, 10.0).text
}

pub fn layout(
  source: &str,
  size: f32,
  color: Color32,
  wrap: f32,
  spacing: f32,
  source_height: f64,
) -> LayoutJob {
  let source = source
    .replace("%%d", "°")
    .replace("%%D", "°")
    .replace("%%p", "±")
    .replace("%%P", "±")
    .replace("%%c", "Ø")
    .replace("%%C", "Ø");
  let chars: Vec<_> = source.chars().collect();
  let mut job = LayoutJob::default();
  job.wrap.max_width = wrap;
  let mut format = Format {
    scale: 1.0,
    color,
    underline: false,
    italic: false,
  };
  let mut stack = Vec::new();
  let mut buffer = String::new();
  let flush = |buffer: &mut String, job: &mut LayoutJob, format: &Format| {
    if buffer.is_empty() {
      return;
    }
    let mut text_format = TextFormat {
      font_id: FontId::proportional(size * format.scale),
      color: readable_color(format.color),
      italics: format.italic,
      line_height: Some(size * format.scale * 1.3 * spacing),
      ..Default::default()
    };
    if format.underline {
      text_format.underline = eframe::egui::Stroke::new(0.7, text_format.color);
    }
    job.append(buffer, 0.0, text_format);
    buffer.clear();
  };
  let mut i = 0;
  while i < chars.len() {
    let c = chars[i];
    i += 1;
    match c {
      '{' => {
        flush(&mut buffer, &mut job, &format);
        stack.push(format.clone());
      }
      '}' => {
        flush(&mut buffer, &mut job, &format);
        if let Some(previous) = stack.pop() {
          format = previous;
        }
      }
      '\\' if i < chars.len() => {
        let command = chars[i];
        i += 1;
        match command {
          'P' | 'X' => buffer.push('\n'),
          '~' => buffer.push('\u{00a0}'),
          '\\' | '{' | '}' => buffer.push(command),
          'L' | 'l' | 'O' | 'o' | 'K' | 'k' => {
            flush(&mut buffer, &mut job, &format);
            if command == 'L' || command == 'l' {
              format.underline = command == 'L';
            }
          }
          'U' if chars.get(i) == Some(&'+') && i + 5 <= chars.len() => {
            let hex: String = chars[i + 1..i + 5].iter().collect();
            if let Some(value) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
              buffer.push(value);
            }
            i += 5;
          }
          'H' | 'C' | 'c' | 'F' | 'f' | 'W' | 'T' | 'Q' | 'A' | 'p' | 'S' => {
            flush(&mut buffer, &mut job, &format);
            let start = i;
            while i < chars.len() && chars[i] != ';' {
              i += 1;
            }
            let value: String = chars[start..i].iter().collect();
            i = (i + 1).min(chars.len());
            match command {
              'H' => {
                if let Ok(scale) = value.trim_end_matches('x').parse::<f32>() {
                  format.scale = if value.ends_with('x') {
                    format.scale * scale
                  } else {
                    scale / source_height.max(0.01) as f32
                  };
                  format.scale = format.scale.clamp(0.05, 20.0);
                }
              }
              'C' => {
                if let Ok(index) = value.parse::<u8>() {
                  format.color = indexed_color(index);
                }
              }
              'c' => {
                if let Ok(rgb) = value.parse::<u32>() {
                  format.color = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                }
              }
              'F' | 'f' => {
                format.italic = value.contains("|i1");
              }
              'S' => {
                buffer.push_str(&value.replace(['#', '^'], "/"));
              }
              _ => {}
            }
          }
          _ => {
            buffer.push('\\');
            buffer.push(command);
          }
        }
      }
      _ => buffer.push(c),
    }
  }
  flush(&mut buffer, &mut job, &format);
  job
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn cad_formatting_does_not_leak_into_the_displayed_text() {
    assert_eq!(
      plain("{\\fArial|b0|i0;Потолок\\P\\C1;Ø12 \\S1/2; \\U+00B0} %%p"),
      "Потолок\nØ12 1/2 ° ±"
    );
  }

  #[test]
  fn absolute_and_relative_text_heights_use_drawing_units() {
    let job = layout(
      "\\H5;A{\\H2x;B}C",
      20.0,
      Color32::BLACK,
      f32::INFINITY,
      1.0,
      10.0,
    );
    let sizes: Vec<_> = job
      .sections
      .iter()
      .map(|section| section.format.font_id.size)
      .collect();
    assert_eq!(sizes, vec![10.0, 20.0, 10.0]);
  }
}
