pub fn rgba(size: u32) -> Vec<u8> {
  assert!((1..=256).contains(&size));
  let mut pixels = Vec::with_capacity((size * size * 4) as usize);
  for y in 0..size {
    for x in 0..size {
      let mut sum = [0_u32; 3];
      let mut covered = 0;
      for sy in 0..4 {
        for sx in 0..4 {
          let px = (x as f32 + (sx as f32 + 0.5) / 4.0) * 64.0 / size as f32;
          let py = (y as f32 + (sy as f32 + 0.5) / 4.0) * 64.0 / size as f32;
          if let Some(color) = color_at(px, py) {
            for channel in 0..3 {
              sum[channel] += u32::from(color[channel]);
            }
            covered += 1;
          }
        }
      }
      for channel in sum {
        pixels.push((channel / covered.max(1)) as u8);
      }
      pixels.push((covered * 255 / 16) as u8);
    }
  }
  pixels
}

fn color_at(x: f32, y: f32) -> Option<[u8; 3]> {
  let dx = ((x - 32.0).abs() - 20.0).max(0.0);
  let dy = ((y - 32.0).abs() - 20.0).max(0.0);
  if dx.hypot(dy) > 9.0 {
    return None;
  }
  let white = [244, 250, 255];
  let mint = [95, 235, 195];
  let hexagon = [
    (22.0, 9.0),
    (42.0, 9.0),
    (50.0, 23.0),
    (42.0, 37.0),
    (22.0, 37.0),
    (14.0, 23.0),
  ];
  for index in 0..6 {
    if on_line((x, y), hexagon[index], hexagon[(index + 1) % 6], 1.3) {
      return Some(white);
    }
  }
  for (cx, cy) in [(26.0, 19.0), (38.0, 19.0), (32.0, 29.0)] {
    if (x - cx).hypot(y - cy) < 2.2 {
      return Some(white);
    }
  }
  for (a, b) in [
    ((14.0, 42.0), (50.0, 42.0)),
    ((14.0, 42.0), (18.0, 40.0)),
    ((14.0, 42.0), (18.0, 44.0)),
    ((50.0, 42.0), (46.0, 40.0)),
    ((50.0, 42.0), (46.0, 44.0)),
  ] {
    if on_line((x, y), a, b, 0.7) {
      return Some(mint);
    }
  }
  // Буквы DXF нарисованы геометрией, без зависимости от установленных шрифтов.
  let glyphs = [
    [
      0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
    ],
    [
      0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
    ],
    [
      0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
    ],
  ];
  for (index, glyph) in glyphs.iter().enumerate() {
    let gx = (x - 13.0 - index as f32 * 14.0) / 2.0;
    let gy = (y - 46.0) / 2.0;
    if (0.0..5.0).contains(&gx)
      && (0.0..7.0).contains(&gy)
      && glyph[gy as usize] & (1 << (4 - gx as usize)) != 0
    {
      return Some(white);
    }
  }
  Some([20, (83.0 - y * 0.45) as u8, (163.0 - y * 0.75) as u8])
}

fn on_line(p: (f32, f32), a: (f32, f32), b: (f32, f32), width: f32) -> bool {
  let (dx, dy) = (b.0 - a.0, b.1 - a.1);
  let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
  (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy) <= width
}
