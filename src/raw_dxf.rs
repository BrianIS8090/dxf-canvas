use std::collections::{BTreeMap, HashMap};

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};

pub fn text_encoding(bytes: &[u8]) -> &'static Encoding {
  if bytes.starts_with(b"AutoCAD Binary DXF") {
    return WINDOWS_1252;
  }
  let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
  let mut lines = bytes.split(|byte| *byte == b'\n');
  let mut in_header = false;
  let mut section_pending = false;
  let mut variable = String::new();
  let mut version = String::new();
  let mut code_page = String::new();
  while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
    let code = std::str::from_utf8(code).unwrap_or("").trim();
    let value = String::from_utf8_lossy(value);
    let value = value.trim();
    if code == "0" && value == "SECTION" {
      section_pending = true;
    } else if code == "2" && section_pending {
      in_header = value == "HEADER";
      section_pending = false;
      if !in_header {
        break;
      }
    } else if code == "0" && value == "ENDSEC" && in_header {
      break;
    } else if in_header {
      if code == "9" {
        variable = value.to_owned();
      } else if variable == "$ACADVER" && code == "1" {
        version = value.to_owned();
      } else if variable == "$DWGCODEPAGE" && code == "3" {
        code_page = value.to_ascii_uppercase();
      }
    }
  }
  // DXF 2007 и новее хранит строки в UTF-8 независимо от исторического DWGCODEPAGE.
  if version
    .strip_prefix("AC")
    .and_then(|number| number.parse::<u32>().ok())
    .is_some_and(|version| version >= 1021)
  {
    return UTF_8;
  }
  let label = match code_page.as_str() {
    "ANSI_932" => "shift_jis".to_owned(),
    "ANSI_936" => "gbk".to_owned(),
    "ANSI_949" => "euc-kr".to_owned(),
    "ANSI_950" => "big5".to_owned(),
    "DOS866" | "DOS_866" => "ibm866".to_owned(),
    value => value.replace("ANSI_", "windows-").replace("UTF_8", "UTF-8"),
  };
  Encoding::for_label(label.as_bytes()).unwrap_or_else(|| {
    if std::str::from_utf8(bytes).is_ok() {
      UTF_8
    } else {
      WINDOWS_1252
    }
  })
}

#[derive(Clone, Debug, Default)]
pub struct Record {
  pub kind: String,
  pub pairs: Vec<(i16, String)>,
}

impl Record {
  pub fn text(&self, code: i16) -> Option<&str> {
    self
      .pairs
      .iter()
      .find(|pair| pair.0 == code)
      .map(|pair| pair.1.as_str())
  }
  pub fn number(&self, code: i16, default: f64) -> f64 {
    self
      .text(code)
      .and_then(|text| text.trim().parse().ok())
      .filter(|v: &f64| v.is_finite())
      .unwrap_or(default)
  }
  pub fn integer(&self, code: i16, default: i32) -> i32 {
    self
      .text(code)
      .and_then(|text| text.trim().parse().ok())
      .unwrap_or(default)
  }
}

#[derive(Default)]
pub struct RawDxf {
  pub layers: HashMap<String, Record>,
  pub extras: HashMap<String, Vec<Record>>,
  pub entity_overrides: HashMap<u64, Record>,
  pub counts: BTreeMap<String, usize>,
  pub binary: bool,
}

impl RawDxf {
  pub fn from_bytes(bytes: &[u8], encoding: &'static Encoding) -> Self {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
      return Self {
        binary: true,
        ..Default::default()
      };
    }
    Self::parse(&encoding.decode(bytes).0)
  }

  pub fn parse(text: &str) -> Self {
    let mut result = Self::default();
    let mut section = String::new();
    let mut block = String::new();
    let mut record = Record::default();
    let mut lines = text.lines();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
      let Ok(code) = code.trim().parse::<i16>() else {
        continue;
      };
      if code == 0 {
        result.accept(&record, &mut section, &mut block);
        record = Record {
          kind: value.trim().to_owned(),
          pairs: Vec::new(),
        };
      } else {
        // Не удерживаем координаты десятков тысяч обычных линий второй раз.
        let retain_all = matches!(
          record.kind.as_str(),
          "HATCH" | "ARC_DIMENSION" | "LAYER" | "SECTION" | "BLOCK"
        );
        if retain_all
          || matches!(code, 5 | 62 | 70 | 420 | 440)
          || (record.kind == "MTEXT" && matches!(code, 11 | 21 | 31 | 50))
        {
          record.pairs.push((code, value.trim_end().to_owned()));
        }
      }
    }
    result.accept(&record, &mut section, &mut block);
    result
  }

  fn accept(&mut self, record: &Record, section: &mut String, block: &mut String) {
    match record.kind.as_str() {
      "SECTION" => {
        *section = record.text(2).unwrap_or("").to_owned();
        return;
      }
      "ENDSEC" => {
        section.clear();
        block.clear();
        return;
      }
      "BLOCK" => {
        *block = record.text(2).unwrap_or("").to_owned();
        return;
      }
      "ENDBLK" => {
        block.clear();
        return;
      }
      _ => {}
    }
    if section == "TABLES" && record.kind == "LAYER" {
      self
        .layers
        .insert(record.text(2).unwrap_or("0").to_owned(), record.clone());
    }
    if section == "ENTITIES" || section == "BLOCKS" {
      if matches!(record.kind.as_str(), "SEQEND" | "VERTEX" | "EOF" | "") {
        return;
      }
      *self.counts.entry(record.kind.clone()).or_default() += 1;
      if matches!(record.kind.as_str(), "HATCH" | "ARC_DIMENSION") {
        self
          .extras
          .entry(block.clone())
          .or_default()
          .push(record.clone());
      } else if (record.kind == "MTEXT" || record.text(420).is_some() || record.text(440).is_some())
        && let Some(handle) = record
          .text(5)
          .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
      {
        self.entity_overrides.insert(handle, record.clone());
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn captures_hatches_and_frozen_layers_that_the_dxf_reader_omits() {
    let raw = RawDxf::parse(
      "0\nSECTION\n2\nTABLES\n0\nLAYER\n2\nHidden\n70\n1\n62\n-3\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nHATCH\n8\nHidden\n91\n1\n0\nENDSEC\n0\nEOF\n",
    );
    assert_eq!(raw.layers["Hidden"].integer(70, 0), 1);
    assert_eq!(raw.extras[""][0].text(8), Some("Hidden"));
    assert_eq!(raw.counts["HATCH"], 1);
  }
}
