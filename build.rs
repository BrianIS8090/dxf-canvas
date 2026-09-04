use std::{env, fs, path::PathBuf, process::Command};

#[path = "src/app_icon.rs"]
mod app_icon;

fn main() {
  println!("cargo:rerun-if-changed=build.rs");
  prepare_converter();
  println!("cargo:rerun-if-changed=src/app_icon.rs");
  for variable in ["RC", "WindowsSdkDir", "ProgramFiles(x86)"] {
    println!("cargo:rerun-if-env-changed={variable}");
  }
  if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
    return;
  }
  assert_eq!(
    env::var("CARGO_CFG_TARGET_ENV").as_deref(),
    Ok("msvc"),
    "Сборка Windows требует MSVC и Windows SDK"
  );
  let output = PathBuf::from(env::var_os("OUT_DIR").expect("Не задан OUT_DIR"));
  let icon_path = output.join("dxf-canvas.ico");
  fs::write(&icon_path, icon_file()).expect("Не удалось создать иконку");
  let resource_path = output.join("dxf-canvas.rc");
  fs::write(
    &resource_path,
    format!(
      "1 ICON \"{}\"\n",
      icon_path.display().to_string().replace('\\', "/")
    ),
  )
  .expect("Не удалось создать описание ресурса");
  let compiled = output.join("dxf-canvas.res");
  let status = Command::new(resource_compiler())
    .arg("/nologo")
    .arg("/fo")
    .arg(&compiled)
    .arg(&resource_path)
    .status()
    .expect("Не удалось запустить компилятор ресурсов Windows SDK");
  assert!(status.success(), "Не удалось встроить иконку приложения");
  println!("cargo:rustc-link-arg-bin=dxf-canvas={}", compiled.display());
}

fn prepare_converter() {
  println!("cargo:rerun-if-env-changed=DXF_CANVAS_DWG_CONVERTER");
  let output = PathBuf::from(env::var_os("OUT_DIR").expect("Не задан OUT_DIR"));
  let destination = output.join("dwg-converter.exe");
  if env::var_os("CARGO_FEATURE_DWG").is_none() {
    fs::write(destination, []).expect("Не удалось подготовить сборку без DWG");
    return;
  }
  assert_eq!(
    env::var("CARGO_CFG_TARGET_OS").as_deref(),
    Ok("windows"),
    "Встроенный DWG-конвертер пока доступен только для Windows"
  );
  assert_eq!(
    env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
    Ok("x86_64"),
    "DWG-конвертер собран для Windows x64"
  );
  let source = env::var_os("DXF_CANVAS_DWG_CONVERTER")
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("target/dwg-converter/DxfCanvas.DwgConverter.exe"));
  println!("cargo:rerun-if-changed={}", source.display());
  let bytes = fs::read(&source).expect("Сначала соберите конвертер: scripts/build-dwg-converter.ps1. Для сборки только DXF используйте --no-default-features");
  assert!(bytes.starts_with(b"MZ"), "Ожидался Windows EXE конвертера");
  fs::write(destination, bytes).expect("Не удалось встроить DWG-конвертер");
}

fn resource_compiler() -> PathBuf {
  if let Some(path) = env::var_os("RC") {
    return path.into();
  }
  let sdk = env::var_os("WindowsSdkDir")
    .map(PathBuf::from)
    .unwrap_or_else(|| {
      PathBuf::from(
        env::var_os("ProgramFiles(x86)").unwrap_or_else(|| "C:\\Program Files (x86)".into()),
      )
      .join("Windows Kits/10")
    });
  let mut candidates: Vec<_> = fs::read_dir(sdk.join("bin"))
    .into_iter()
    .flatten()
    .filter_map(Result::ok)
    .map(|entry| entry.path().join("x64/rc.exe"))
    .filter(|path| path.is_file())
    .collect();
  candidates.sort();
  candidates
    .pop()
    .expect("Не найден rc.exe. Установите Windows SDK или задайте путь в RC")
}

fn icon_file() -> Vec<u8> {
  let sizes = [16_u32, 20, 24, 32, 40, 48, 64, 128, 256];
  let mut result = Vec::from([0, 0, 1, 0, sizes.len() as u8, 0]);
  let mut images = Vec::new();
  let mut offset = 6 + 16 * sizes.len() as u32;
  for size in sizes {
    let pixels = app_icon::rgba(size);
    let mask_stride = size.div_ceil(32) * 4;
    let mut bitmap = Vec::new();
    for value in [40, size, size * 2] {
      bitmap.extend(value.to_le_bytes());
    }
    bitmap.extend([1, 0, 32, 0]);
    for value in [0, size * size * 4, 0, 0, 0, 0] {
      bitmap.extend(value.to_le_bytes());
    }
    // ICO хранит строки снизу вверх, каналы в порядке BGRA, затем маску прозрачности.
    for y in (0..size).rev() {
      for x in 0..size {
        let i = ((y * size + x) * 4) as usize;
        bitmap.extend([pixels[i + 2], pixels[i + 1], pixels[i], pixels[i + 3]]);
      }
    }
    for y in (0..size).rev() {
      let mut mask = vec![0_u8; mask_stride as usize];
      for x in 0..size {
        if pixels[((y * size + x) * 4 + 3) as usize] == 0 {
          mask[(x / 8) as usize] |= 0x80 >> (x % 8);
        }
      }
      bitmap.extend(mask);
    }
    result.extend([size as u8, size as u8, 0, 0, 1, 0, 32, 0]);
    result.extend((bitmap.len() as u32).to_le_bytes());
    result.extend(offset.to_le_bytes());
    offset += bitmap.len() as u32;
    images.extend(bitmap);
  }
  result.extend(images);
  result
}
