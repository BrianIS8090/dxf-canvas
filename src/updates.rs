use std::{
  io::Write,
  path::{Path, PathBuf},
  sync::mpsc::{self, Receiver},
  time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eframe::egui;
use semver::Version;
use serde::{Deserialize, Serialize};

const API: &str = "https://api.github.com/repos/BrianIS8090/dxf-canvas/releases/latest";
const RELEASES: &str = "https://github.com/BrianIS8090/dxf-canvas/releases";
const CHECK_INTERVAL: u64 = 24 * 60 * 60;
const MAX_RESPONSE: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct AvailableRelease {
  pub version: String,
  pub page_url: String,
  pub download_url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
  disabled: bool,
  last_attempt: u64,
}

#[derive(Deserialize)]
struct Release {
  tag_name: String,
  draft: bool,
  prerelease: bool,
  assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
  name: String,
  browser_download_url: String,
  state: String,
}

fn parse_release(json: &str, current: &str) -> Result<Option<AvailableRelease>, String> {
  let release: Release = serde_json::from_str(json).map_err(|_| {
    "GitHub вернул непонятное описание выпуска. Повторите проверку позже.".to_owned()
  })?;
  let tag = release
    .tag_name
    .strip_prefix('v')
    .unwrap_or(&release.tag_name);
  let version = Version::parse(tag).map_err(|_| "Неверная версия выпуска на GitHub.")?;
  let current = Version::parse(current).map_err(|_| "Неверная версия приложения.")?;
  if release.draft || release.prerelease || !version.pre.is_empty() || version <= current {
    return Ok(None);
  }
  // Ссылки строятся только для нашего репозитория; произвольные адреса из ответа не открываем.
  if release.tag_name != format!("v{version}") || !version.build.is_empty() {
    return Err("Неожиданный формат тега выпуска.".into());
  }
  let name = format!("DXF-Canvas-{version}-setup-x64.exe");
  let download_url = format!("{RELEASES}/download/v{version}/{name}");
  if !release.assets.iter().any(|asset| {
    asset.name == name && asset.state == "uploaded" && asset.browser_download_url == download_url
  }) {
    return Err("Новый выпуск найден, но установщик ещё не готов. Проверьте позже.".into());
  }
  Ok(Some(AvailableRelease {
    version: version.to_string(),
    page_url: format!("{RELEASES}/tag/v{version}"),
    download_url,
  }))
}

fn fetch_release() -> Result<Option<AvailableRelease>, String> {
  let config = ureq::Agent::config_builder()
    .https_only(true)
    .max_redirects(0)
    .timeout_global(Some(Duration::from_secs(15)))
    .build();
  let agent: ureq::Agent = config.into();
  let mut response = agent
    .get(API)
    .header(
      "User-Agent",
      concat!("DXF-Canvas/", env!("CARGO_PKG_VERSION")),
    )
    .header("Accept", "application/vnd.github+json")
    .header("X-GitHub-Api-Version", "2022-11-28")
    .call()
    .map_err(|error| match error {
      ureq::Error::StatusCode(403 | 429) => {
        "GitHub временно ограничил запросы. Повторите позже.".to_owned()
      }
      _ => "Не удалось связаться с GitHub. Проверьте интернет и повторите попытку.".to_owned(),
    })?;
  if response.status() != 200 {
    return Err("Неожиданный ответ GitHub. Повторите проверку позже.".into());
  }
  let json = response
    .body_mut()
    .with_config()
    .limit(MAX_RESPONSE)
    .read_to_string()
    .map_err(|_| "Не удалось прочитать описание выпуска GitHub.".to_owned())?;
  parse_release(&json, env!("CARGO_PKG_VERSION"))
}

fn preference_path() -> Option<PathBuf> {
  std::env::var_os("LOCALAPPDATA")
    .filter(|value| !value.is_empty())
    .map(|path| PathBuf::from(path).join("DXF-Canvas/updates.json"))
}

fn read_preferences(path: &Path) -> Preferences {
  std::fs::read(path)
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default()
}

fn write_preferences(path: &Path, preferences: &Preferences) -> std::io::Result<()> {
  let parent = path.parent().unwrap_or_else(|| Path::new("."));
  std::fs::create_dir_all(parent)?;
  let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
  temporary.write_all(&serde_json::to_vec(preferences)?)?;
  temporary.as_file().sync_all()?;
  temporary.persist(path).map_err(|error| error.error)?;
  Ok(())
}

fn now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn check_due(preferences: &Preferences, timestamp: u64) -> bool {
  !preferences.disabled
    && (preferences.last_attempt == 0
      || timestamp < preferences.last_attempt
      || timestamp - preferences.last_attempt >= CHECK_INTERVAL)
}

pub struct Updates {
  pub open: bool,
  preferences: Preferences,
  path: Option<PathBuf>,
  started: Instant,
  startup_checked: bool,
  receiver: Option<Receiver<Result<Option<AvailableRelease>, String>>>,
  result: Option<Result<Option<AvailableRelease>, String>>,
  preference_error: Option<String>,
}

impl Default for Updates {
  fn default() -> Self {
    Self {
      open: false,
      preferences: Preferences::default(),
      path: None,
      started: Instant::now(),
      startup_checked: true,
      receiver: None,
      result: None,
      preference_error: None,
    }
  }
}

impl Updates {
  pub fn load() -> Self {
    let path = preference_path();
    Self {
      preferences: path.as_deref().map(read_preferences).unwrap_or_default(),
      path,
      startup_checked: false,
      ..Self::default()
    }
  }

  pub fn available(&self) -> bool {
    matches!(self.result, Some(Ok(Some(_))))
  }

  fn save_preferences(&mut self) {
    self.preference_error = self
      .path
      .as_deref()
      .and_then(|path| write_preferences(path, &self.preferences).err())
      .map(|_| "Не удалось сохранить настройку обновлений.".into());
  }

  fn start(&mut self, context: &egui::Context) {
    if self.receiver.is_some() {
      return;
    }
    self.preferences.last_attempt = now();
    self.save_preferences();
    self.startup_checked = true;
    let (sender, receiver) = mpsc::channel();
    let context = context.clone();
    self.receiver = Some(receiver);
    // Сеть работает отдельно от интерфейса и не получает пути или содержимое чертежей.
    std::thread::spawn(move || {
      let _ = sender.send(fetch_release());
      context.request_repaint();
    });
  }

  pub fn poll(&mut self, context: &egui::Context) {
    if !self.startup_checked {
      if self.started.elapsed() < Duration::from_secs(5) {
        context.request_repaint_after(Duration::from_secs(1));
      } else {
        self.startup_checked = true;
        if check_due(&self.preferences, now()) {
          self.start(context);
        }
      }
    }
    if let Some(receiver) = &self.receiver {
      match receiver.try_recv() {
        Ok(result) => {
          self.result = Some(result);
          self.receiver = None;
        }
        Err(mpsc::TryRecvError::Disconnected) => {
          self.result = Some(Err("Проверка прервана. Повторите попытку.".into()));
          self.receiver = None;
        }
        Err(mpsc::TryRecvError::Empty) => {}
      }
    }
  }

  pub fn show(&mut self, context: &egui::Context) {
    let mut open = self.open;
    egui::Window::new("Обновления DXF Холст")
      .open(&mut open).collapsible(false).resizable(false).default_width(430.0)
      .show(context, |ui| {
        ui.label(format!("Установленная версия: {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(8.0);
        if self.receiver.is_some() {
          ui.horizontal(|ui| { ui.spinner(); ui.label("Проверяю новые версии…"); });
        } else {
          match &self.result {
            Some(Ok(Some(release))) => {
              ui.heading(format!("Доступна версия {}", release.version));
              ui.hyperlink_to("Скачать установщик", &release.download_url);
              ui.hyperlink_to("Что изменилось", &release.page_url);
              ui.label("Завершите работу с холстом и запустите скачанный установщик. Расстановка и измерения текущего окна не сохраняются при закрытии.");
            }
            Some(Ok(None)) => { ui.label("У вас актуальная версия. Новых стабильных выпусков нет."); }
            Some(Err(message)) => { ui.colored_label(ui.visuals().warn_fg_color, message); }
            None => { ui.label("Проверка опубликованных выпусков на GitHub."); }
          }
        }
        ui.add_space(8.0);
        if ui.add_enabled(self.receiver.is_none(), egui::Button::new("Проверить сейчас")).clicked() {
          self.start(context);
        }
        ui.separator();
        let mut automatic = !self.preferences.disabled;
        if ui.checkbox(&mut automatic, "Проверять при запуске, не чаще раза в сутки").changed() {
          self.preferences.disabled = !automatic;
          self.save_preferences();
        }
        ui.small("Передаётся только запрос версии. Чертежи остаются на компьютере. Загрузка и установка — по вашему нажатию.");
        if let Some(error) = &self.preference_error { ui.colored_label(ui.visuals().warn_fg_color, error); }
      });
    self.open = open;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn release(version: &str) -> serde_json::Value {
    json!({"tag_name": format!("v{version}"), "draft": false, "prerelease": false,
      "assets": [{"name": format!("DXF-Canvas-{version}-setup-x64.exe"), "state": "uploaded",
        "browser_download_url": format!("{RELEASES}/download/v{version}/DXF-Canvas-{version}-setup-x64.exe")} ]})
  }

  #[test]
  fn compares_versions_numerically_and_never_downgrades() {
    assert!(
      parse_release(&release("0.10.0").to_string(), "0.9.9")
        .unwrap()
        .is_some()
    );
    for version in ["0.9.6", "0.9.5", "0.8.9"] {
      assert!(
        parse_release(&release(version).to_string(), "0.9.6")
          .unwrap()
          .is_none()
      );
    }
  }

  #[test]
  fn ignores_drafts_and_preliminary_releases() {
    for field in ["draft", "prerelease"] {
      let mut value = release("0.10.0");
      value[field] = json!(true);
      assert!(
        parse_release(&value.to_string(), "0.9.6")
          .unwrap()
          .is_none()
      );
    }
    assert!(
      parse_release(&release("0.10.0-alpha.1").to_string(), "0.9.6")
        .unwrap()
        .is_none()
    );
  }

  #[test]
  fn requires_complete_installer_on_the_exact_project_url() {
    for url in [
      "http://github.com/file.exe",
      "https://github.com.evil.test/file.exe",
      "file:///evil.exe",
      "https://github.com/other/repo/releases/file.exe",
    ] {
      let mut value = release("0.10.0");
      value["assets"][0]["browser_download_url"] = json!(url);
      assert!(parse_release(&value.to_string(), "0.9.6").is_err());
    }
    let mut value = release("0.10.0");
    value["assets"][0]["state"] = json!("new");
    assert!(parse_release(&value.to_string(), "0.9.6").is_err());
    value["assets"] = json!([]);
    assert!(parse_release(&value.to_string(), "0.9.6").is_err());
  }

  #[test]
  fn malformed_response_does_not_claim_the_app_is_current() {
    for json in ["garbage", "{}", "[]", "{\"message\":\"rate limited\"}"] {
      assert!(parse_release(json, "0.9.6").is_err());
    }
    assert!(parse_release(&release("bogus").to_string(), "0.9.6").is_err());
  }

  #[test]
  fn daily_check_respects_opt_out_and_clock_changes() {
    let mut prefs = Preferences::default();
    assert!(check_due(&prefs, 100));
    prefs.last_attempt = 100;
    assert!(!check_due(&prefs, 101));
    assert!(check_due(&prefs, 100 + CHECK_INTERVAL));
    assert!(check_due(&prefs, 99));
    prefs.disabled = true;
    assert!(!check_due(&prefs, 100 + CHECK_INTERVAL));
  }

  #[test]
  fn preferences_persist_in_a_separate_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings/updates.json");
    assert!(!read_preferences(&path).disabled);
    write_preferences(
      &path,
      &Preferences {
        disabled: true,
        last_attempt: 42,
      },
    )
    .unwrap();
    let prefs = read_preferences(&path);
    assert!(prefs.disabled);
    assert_eq!(prefs.last_attempt, 42);
    write_preferences(&path, &Preferences::default()).unwrap();
    assert!(!read_preferences(&path).disabled);
  }

  #[test]
  fn background_result_does_not_open_a_dialog() {
    let mut updates = Updates::default();
    let (sender, receiver) = mpsc::channel();
    updates.receiver = Some(receiver);
    sender
      .send(parse_release(&release("0.10.0").to_string(), "0.9.6"))
      .unwrap();
    updates.poll(&egui::Context::default());
    assert!(updates.available());
    assert!(!updates.open);
    assert!(updates.receiver.is_none());
  }

  #[test]
  fn updates_window_renders_all_states_in_both_themes() {
    for dark in [false, true] {
      for result in [
        None,
        Some(Ok(None)),
        Some(Err("Нет интернета".into())),
        Some(parse_release(&release("0.10.0").to_string(), "0.9.6")),
      ] {
        let mut updates = Updates {
          open: true,
          result,
          ..Updates::default()
        };
        let context = egui::Context::default();
        crate::theme::configure(&context);
        crate::theme::apply(&context, dark);
        for _ in 0..2 {
          let mut output = context.run_ui(egui::RawInput::default(), |_| updates.show(&context));
          assert!(output.platform_output.commands.is_empty());
          output.textures_delta.clear();
        }
        assert!(updates.open);
      }
    }
  }

  #[test]
  #[ignore = "Нужен доступ к публичному API GitHub"]
  fn live_release_check() {
    assert!(fetch_release().is_ok());
  }
}
