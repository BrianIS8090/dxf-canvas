use std::{
  fs::{self, File},
  io::Read,
  path::Path,
  process::{Child, Command, Stdio},
  time::{Duration, Instant},
};

use crate::{dxf_import::load_dxf, geometry::DrawingItem};

const CONVERTER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dwg-converter.exe"));
const TIMEOUT: Duration = Duration::from_secs(180);

pub fn is_dwg(path: &Path) -> bool {
  path
    .extension()
    .is_some_and(|extension| extension.eq_ignore_ascii_case("dwg"))
}

pub fn load_drawing(path: &Path, phase: impl Fn(&str)) -> Result<DrawingItem, String> {
  if !is_dwg(path) {
    phase("Чтение DXF и подготовка чертежа…");
    return load_dxf(path).map_err(|error| error.to_string());
  }
  phase("Преобразование DWG → DXF…");
  let mut header = [0_u8; 6];
  File::open(path)
    .and_then(|mut file| file.read_exact(&mut header))
    .map_err(|error| format!("Не удалось прочитать DWG: {error}"))?;
  validate_version(&header)?;
  if CONVERTER.is_empty() {
    return Err(
      "В этой сборке нет DWG-конвертера. Используйте полную Windows x64 версию приложения.".into(),
    );
  }
  let directory = tempfile::Builder::new()
    .prefix("dxf-canvas-dwg-")
    .tempdir()
    .map_err(|error| format!("Не удалось создать временную папку: {error}"))?;
  let executable = directory.path().join("converter.exe");
  let converted = directory.path().join("drawing.dxf");
  fs::write(&executable, CONVERTER).map_err(|error| error.to_string())?;
  // Виртуальные диски могут читать файл, но не поддерживать запрос физического пути тома.
  // Конвертеру достаточно абсолютного пути, без canonicalize и обращения к драйверу тома.
  let input = std::path::absolute(path)
    .map_err(|error| format!("Не удалось определить полный путь DWG: {error}"))?;
  let output =
    File::create(directory.path().join("report.json")).map_err(|error| error.to_string())?;
  let errors =
    File::create(directory.path().join("error.txt")).map_err(|error| error.to_string())?;
  let mut command = Command::new(&executable);
  command
    .arg(&input)
    .arg(&converted)
    .current_dir(directory.path())
    .stdin(Stdio::null())
    .stdout(output)
    .stderr(errors);
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
  }
  let mut child = ChildGuard(
    command
      .spawn()
      .map_err(|error| format!("Не удалось запустить DWG-конвертер: {error}"))?,
  );
  #[cfg(windows)]
  let _job = WindowsJob::attach(&child.0)?;
  let started = Instant::now();
  let status = loop {
    if let Some(status) = child.0.try_wait().map_err(|error| error.to_string())? {
      break status;
    }
    if started.elapsed() >= TIMEOUT {
      return Err("Преобразование DWG остановлено: превышено время ожидания 3 минуты.".into());
    }
    std::thread::sleep(Duration::from_millis(25));
  };
  if !status.success() {
    let detail = bounded_text(&directory.path().join("error.txt"), 16_000)?;
    return Err(format!("Не удалось преобразовать DWG ({status}). {detail}"));
  }
  // JSON может экранировать каждый символ предупреждения шестью ASCII-байтами.
  let report = bounded_text(&directory.path().join("report.json"), 640_000)?;
  let warnings = conversion_warnings(&report)?;
  phase("Чтение преобразованного DXF и подготовка чертежа…");
  let mut item = load_dxf(&converted)
    .map_err(|error| format!("Не удалось открыть результат DWG-конвертации: {error}"))?;
  // Холст сохраняет имя и путь исходного DWG, а не временного DXF.
  item.path = path.to_path_buf();
  item.name = path
    .file_stem()
    .ok_or("Не удалось определить имя DWG")?
    .to_string_lossy()
    .trim()
    .to_owned();
  item.appearance.warnings.extend(warnings);
  // Конвертер уже закончил работу, поэтому временные данные удаляются вместе с directory.
  Ok(item)
}

fn validate_version(header: &[u8; 6]) -> Result<(), String> {
  if [
    b"AC1014", b"AC1015", b"AC1018", b"AC1021", b"AC1024", b"AC1027", b"AC1032",
  ]
  .contains(&header)
  {
    Ok(())
  } else {
    Err(format!(
      "Неподдерживаемый или повреждённый DWG (заголовок {}). Поддерживаются R14–2018/AC1032; для старого файла сохраните DXF в CAD-системе.",
      String::from_utf8_lossy(header)
    ))
  }
}

fn bounded_text(path: &Path, limit: u64) -> Result<String, String> {
  let mut bytes = Vec::new();
  File::open(path)
    .and_then(|file| file.take(limit).read_to_end(&mut bytes))
    .map_err(|error| error.to_string())?;
  Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn conversion_warnings(report: &str) -> Result<Vec<String>, String> {
  let report: serde_json::Value = serde_json::from_str(report)
    .map_err(|error| format!("Некорректный отчёт DWG-конвертера: {error}"))?;
  if report["engine"] != "ACadSharp 3.7.1" {
    return Err("Неожиданная версия встроенного DWG-конвертера".into());
  }
  let count = report["warning_count"]
    .as_u64()
    .ok_or("В отчёте конвертера нет числа предупреждений")?;
  let details = report["warnings"]
    .as_array()
    .ok_or("В отчёте конвертера нет предупреждений")?;
  let mut warnings = Vec::new();
  if count > 0 {
    warnings.push(format!("DWG преобразован с предупреждениями: {count}. Проверьте результат по исходному чертежу; подробности конвертера ниже."));
    warnings.extend(
      details
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::to_owned),
    );
    if count > details.len() as u64 {
      warnings.push(format!(
        "Ещё {} предупреждений не показано (предел подробного отчёта — 100).",
        count - details.len() as u64
      ));
    }
  }
  Ok(warnings)
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
  fn drop(&mut self) {
    let _ = self.0.kill();
    let _ = self.0.wait();
  }
}

#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl WindowsJob {
  fn attach(child: &Child) -> Result<Self, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::*;
    // Windows завершает дочерний конвертер при закрытии основного приложения.
    unsafe {
      let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
      if handle.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
      }
      let job = Self(handle);
      let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
      limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
      if SetInformationJobObject(
        handle,
        JobObjectExtendedLimitInformation,
        (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
        std::mem::size_of_val(&limits) as u32,
      ) == 0
        || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
      {
        return Err(format!(
          "Не удалось изолировать процесс конвертера: {}",
          std::io::Error::last_os_error()
        ));
      }
      Ok(job)
    }
  }
}

#[cfg(windows)]
impl Drop for WindowsJob {
  fn drop(&mut self) {
    unsafe {
      windows_sys::Win32::Foundation::CloseHandle(self.0);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn versions_and_extensions_are_checked_before_conversion() {
    assert!(is_dwg(Path::new("русское имя.DWG")));
    assert!(!is_dwg(Path::new("a.dxf")));
    for version in [b"AC1014", b"AC1032"] {
      assert!(validate_version(version).is_ok());
    }
    for version in [b"AC1009", b"AC9999", b"broken"] {
      assert!(validate_version(version).is_err());
    }
  }

  #[test]
  fn conversion_warnings_are_not_silently_discarded() {
    assert!(
      conversion_warnings(r#"{"engine":"ACadSharp 3.7.1","warning_count":0,"warnings":[]}"#)
        .unwrap()
        .is_empty()
    );
    let warnings = conversion_warnings(
      r#"{"engine":"ACadSharp 3.7.1","warning_count":2,"warnings":["Unknown object"]}"#,
    )
    .unwrap();
    assert_eq!(warnings.len(), 3);
    assert!(warnings[1].contains("Unknown object"));
    assert!(conversion_warnings("not json").is_err());
  }

  #[test]
  fn damaged_input_is_not_changed() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("повреждённый.dwg");
    fs::write(&input, b"broken").unwrap();
    assert!(load_drawing(&input, |_| {}).is_err());
    assert_eq!(fs::read(input).unwrap(), b"broken");
  }

  #[test]
  #[cfg(all(windows, feature = "dwg"))]
  fn embedded_converter_preserves_geometry_text_layers_and_source_name() {
    let directory = tempfile::tempdir().unwrap();
    let generator = directory.path().join("generator.exe");
    fs::write(&generator, CONVERTER).unwrap();
    let input = directory.path().join("Пластина 160 х 100.dwg");
    let status = Command::new(&generator)
      .arg("--write-test-dwg")
      .arg(&input)
      .status()
      .unwrap();
    assert!(status.success());
    let before = fs::read(&input).unwrap();
    let mut phases = Vec::new();
    let phases = std::sync::Mutex::new(&mut phases);
    let item = load_drawing(&input, |phase| {
      phases.lock().unwrap().push(phase.to_owned())
    })
    .unwrap();
    assert_eq!(item.path, input);
    assert_eq!(item.name, "Пластина 160 х 100");
    assert_eq!(item.primitives.len(), 5);
    // Генератор также создаёт видовой экран листа: он не поддерживается, но не должен теряться без предупреждения.
    assert_eq!(item.unsupported_entities, 1);
    assert!(
      item
        .appearance
        .warnings
        .iter()
        .any(|warning| warning.contains("VIEWPORT"))
    );
    assert!(
      item
        .appearance
        .layers
        .iter()
        .any(|layer| layer.name == "Контур")
    );
    assert_eq!(item.appearance.texts[0].text, "Тест DWG");
    assert!((item.bounds.width() - 160.0).abs() < 1.0e-6);
    assert!((item.bounds.height() - 100.0).abs() < 1.0e-6);
    assert_eq!(fs::read(input).unwrap(), before);
    assert_eq!(phases.lock().unwrap().len(), 2);

    let broken = directory.path().join("повреждённый AC1032.dwg");
    fs::write(&broken, b"AC1032\0\0\0\0").unwrap();
    let error = load_drawing(&broken, |_| {}).unwrap_err();
    assert!(error.contains("Не удалось преобразовать DWG"), "{error}");
    assert_eq!(fs::read(&broken).unwrap(), b"AC1032\0\0\0\0");
  }

  #[test]
  #[ignore = "Нужны локальные DWG_REFERENCE_FIXTURE и DXF_REFERENCE_FIXTURE"]
  fn reference_dwg_matches_reference_dxf() {
    let input = std::env::var_os("DWG_REFERENCE_FIXTURE").expect("Не задан DWG_REFERENCE_FIXTURE");
    let reference =
      std::env::var_os("DXF_REFERENCE_FIXTURE").expect("Не задан DXF_REFERENCE_FIXTURE");
    let started = Instant::now();
    let actual = load_drawing(Path::new(&input), |_| {}).unwrap();
    let expected = load_dxf(Path::new(&reference)).unwrap();
    assert_eq!(actual.units.is_known(), expected.units.is_known());
    assert_eq!(actual.units.factor(), expected.units.factor());
    assert_eq!(actual.primitives.len(), expected.primitives.len());
    assert_eq!(
      actual.appearance.texts.len(),
      expected.appearance.texts.len()
    );
    assert_eq!(
      actual.appearance.fills.len(),
      expected.appearance.fills.len()
    );
    assert_eq!(actual.unsupported_entities, 0);
    let layers = |item: &DrawingItem| {
      let mut layers: Vec<_> = item
        .appearance
        .layers
        .iter()
        .map(|layer| (layer.name.clone(), layer.color, layer.initial_visible))
        .collect();
      layers.sort_by(|a, b| a.0.cmp(&b.0));
      layers
    };
    assert_eq!(layers(&actual), layers(&expected));
    let text = |item: &DrawingItem| {
      let mut strings: Vec<_> = item
        .appearance
        .texts
        .iter()
        .map(|text| text.text.clone())
        .collect();
      strings.sort();
      strings
    };
    assert_eq!(text(&actual), text(&expected));
    let delta = (actual.bounds.min.x - expected.bounds.min.x)
      .abs()
      .max((actual.bounds.min.y - expected.bounds.min.y).abs())
      .max((actual.bounds.max.x - expected.bounds.max.x).abs())
      .max((actual.bounds.max.y - expected.bounds.max.y).abs());
    assert!(delta < 0.02, "Изменились общие габариты: {delta}");
    let mut matched = vec![false; expected.primitives.len()];
    for (index, bounds) in actual.appearance.primitive_bounds.iter().enumerate() {
      let candidates = expected
        .appearance
        .render_index
        .query(crate::spatial::neighborhood(
          bounds.center(),
          bounds.width().max(bounds.height()) * 0.5 + 0.02,
        ));
      let found = candidates.into_iter().find(|&i| {
        if matched[i] {
          return false;
        }
        let other = expected.appearance.primitive_bounds[i];
        (bounds.min.x - other.min.x).abs() < 0.02
          && (bounds.min.y - other.min.y).abs() < 0.02
          && (bounds.max.x - other.max.x).abs() < 0.02
          && (bounds.max.y - other.max.y).abs() < 0.02
          && same_display_geometry(&actual.primitives[index], &expected.primitives[i])
          && actual.appearance.styles[index].color == expected.appearance.styles[i].color
          && actual.appearance.layers[actual.appearance.styles[index].layer].name
            == expected.appearance.layers[expected.appearance.styles[i].layer].name
      });
      let i =
        found.unwrap_or_else(|| panic!("Не найден соответствующий элемент {index}: {bounds:?}"));
      matched[i] = true;
    }
    println!(
      "DWG: {} примитивов, {} текстов, {} заливок, {} слоёв; разница габаритов {delta:.9}; проверка {:?}",
      actual.primitives.len(),
      actual.appearance.texts.len(),
      actual.appearance.fills.len(),
      actual.appearance.layers.len(),
      started.elapsed()
    );
  }

  #[test]
  #[ignore = "Нужен локальный DWG_LAYER_FIXTURE; производственный файл не публикуется"]
  fn bath_hall_dwg_preserves_cyrillic_layers() {
    let path = std::env::var_os("DWG_LAYER_FIXTURE").expect("Задайте DWG_LAYER_FIXTURE");
    let item = load_drawing(Path::new(&path), |_| {}).unwrap();
    assert_eq!(item.appearance.layers.len(), 84);
    assert!(
      item
        .appearance
        .layers
        .iter()
        .any(|layer| layer.name.starts_with("Новый_"))
    );
    assert!(
      item
        .appearance
        .layers
        .iter()
        .all(|layer| !layer.name.chars().any(|ch| ('À'..='ÿ').contains(&ch))),
      "Остались искажённые названия слоёв"
    );
    assert!(
      item
        .appearance
        .layers
        .iter()
        .any(|layer| layer.name.chars().any(|ch| ('А'..='я').contains(&ch))),
      "Вместо кириллицы в слоях: {:?}",
      item
        .appearance
        .layers
        .iter()
        .take(20)
        .map(|layer| &layer.name)
        .collect::<Vec<_>>()
    );
    println!(
      "Проверено слоёв: {}; примеры: {:?}",
      item.appearance.layers.len(),
      item
        .appearance
        .layers
        .iter()
        .take(4)
        .map(|layer| &layer.name)
        .collect::<Vec<_>>()
    );
  }

  #[test]
  #[ignore = "Нужен доступный DWG_VIRTUAL_FIXTURE на подключённом виртуальном диске"]
  fn virtual_drive_dwg_opens_without_modifying_source() {
    let path = std::env::var_os("DWG_VIRTUAL_FIXTURE").expect("Задайте DWG_VIRTUAL_FIXTURE");
    let path = Path::new(&path);
    let before = fs::read(path).expect("Не удалось прочитать исходный файл");
    let result = load_drawing(path, |_| {});
    assert_eq!(fs::read(path).unwrap(), before);
    let item = result.expect("Файл читается с диска, но импорт DWG завершается ошибкой");
    assert_eq!(item.path, path);
    assert_eq!(
      item.name,
      path.file_stem().unwrap().to_string_lossy().trim()
    );
    assert!(!item.primitives.is_empty());
  }

  fn same_display_geometry(a: &crate::geometry::Primitive, b: &crate::geometry::Primitive) -> bool {
    use crate::geometry::{Point, Primitive};
    let near = |a: &Point, b: &Point| (a.x - b.x).hypot(a.y - b.y) < 0.0001;
    match (a, b) {
      (Primitive::Point(a), Primitive::Point(b)) => near(a, b),
      (
        Primitive::Path {
          points: a,
          closed: ac,
          ..
        },
        Primitive::Path {
          points: b,
          closed: bc,
          ..
        },
      ) => ac == bc && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| near(a, b)),
      _ => false,
    }
  }
}
