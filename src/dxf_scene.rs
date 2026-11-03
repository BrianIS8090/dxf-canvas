use std::{collections::HashMap, sync::Arc};

use dxf::{
  Drawing,
  entities::{Entity, EntityCommon, EntityType},
};
use eframe::egui::Color32;

use crate::{
  cad_scene::{
    Appearance, CadFill, CadPatternHatch, CadText, DrawStep, EntityStyle, Layer, indexed_color,
  },
  dxf_import::{Transform2, append_entity},
  elevation::{ElevationObject, Transform3, ZRange},
  geometry::{Bounds, Point, Primitive},
  raw_dxf::{RawDxf, Record},
};

pub fn extract(drawing: &Drawing, raw: &RawDxf) -> (Vec<Primitive>, Appearance, usize) {
  // Сопоставляем также MTEXT без handle; повторные вставки блока используют одну запись.
  let mut mtexts = HashMap::new();
  let mut index_texts = |name: &str, entities: Vec<&Entity>| {
    if let Some(records) = raw.mtexts.get(name) {
      for (entity, record) in entities
        .into_iter()
        .filter(|entity| matches!(entity.specific, EntityType::MText(_)))
        .zip(records)
      {
        mtexts.insert(entity as *const Entity as usize, record);
      }
    }
  };
  index_texts("", drawing.entities().collect());
  for block in drawing.blocks() {
    index_texts(&block.name, block.entities.iter().collect());
  }
  let mut wipeouts = HashMap::new();
  let mut index_masks = |name: &str, entities: Vec<&Entity>| {
    if let Some(records) = raw.wipeouts.get(name) {
      for (entity, record) in entities
        .into_iter()
        .filter(|e| matches!(e.specific, EntityType::Wipeout(_)))
        .zip(records)
      {
        wipeouts.insert(entity as *const Entity as usize, record);
      }
    }
  };
  index_masks("", drawing.entities().collect());
  for block in drawing.blocks() {
    index_masks(&block.name, block.entities.iter().collect());
  }
  let mut builder = Builder {
    drawing,
    raw,
    geometry: Vec::new(),
    appearance: Appearance::default(),
    unsupported: 0,
    layer_ids: HashMap::new(),
    mtexts,
    wipeouts,
    unsupported_types: HashMap::new(),
    source_kinds: raw
      .sequence
      .values()
      .flatten()
      .filter(|(_, h)| *h != 0)
      .map(|(k, h)| (*h, k.as_str()))
      .collect(),
    elevation_transform: Transform3::IDENTITY,
    block_path: String::new(),
  };
  for layer in drawing.layers() {
    builder.layer(&layer.name);
  }
  if builder.appearance.layers.is_empty() {
    builder.layer("0");
  }
  builder.appearance.source_counts = raw.counts.clone();
  if raw.binary {
    builder.appearance.warnings.push("Бинарный DXF: дополнительные сущности HATCH/ARC_DIMENSION и часть свойств слоёв не прочитаны. Для полного отображения сохраните ASCII DXF.".to_owned());
  }
  builder.contents(
    "",
    drawing.entities().collect(),
    Transform2::IDENTITY,
    None,
    0,
    true,
  );
  for (kind, count) in &builder.unsupported_types {
    let source_count = raw.counts.get(kind).copied().unwrap_or(*count);
    builder.appearance.warnings.push(format!("Не отображается {kind}: {count} экземпляров; в исходном DXF {source_count} записей, включая определения блоков."));
  }
  // Дополняем счётчик типами, которые библиотека DXF вообще не передаёт вызывающему коду.
  for (kind, count) in &raw.counts {
    if !matches!(
      kind.as_str(),
      "3DFACE"
        | "3DSOLID"
        | "ACAD_PROXY_ENTITY"
        | "ARC"
        | "ARCALIGNEDTEXT"
        | "ATTDEF"
        | "ATTRIB"
        | "BODY"
        | "CIRCLE"
        | "DIMENSION"
        | "ELLIPSE"
        | "HELIX"
        | "IMAGE"
        | "INSERT"
        | "LEADER"
        | "LIGHT"
        | "LINE"
        | "3DLINE"
        | "LWPOLYLINE"
        | "MLINE"
        | "MTEXT"
        | "OLEFRAME"
        | "OLE2FRAME"
        | "POINT"
        | "POLYLINE"
        | "RAY"
        | "REGION"
        | "RTEXT"
        | "SECTION"
        | "SEQEND"
        | "SHAPE"
        | "SOLID"
        | "SPLINE"
        | "TEXT"
        | "TOLERANCE"
        | "TRACE"
        | "DGNUNDERLAY"
        | "DWFUNDERLAY"
        | "PDFUNDERLAY"
        | "VERTEX"
        | "WIPEOUT"
        | "XLINE"
        | "HATCH"
        | "ARC_DIMENSION"
    ) {
      builder.warning(&format!("Неподдерживаемый тип {kind}: {count}"));
      builder.unsupported += count.saturating_sub(1);
    }
  }
  for (primitive, style) in builder.geometry.iter().zip(&builder.appearance.styles) {
    builder
      .appearance
      .primitive_bounds
      .push(primitive.bounds().unwrap_or_else(Bounds::empty));
    builder.appearance.layers[style.layer].count += 1;
  }
  for text in &builder.appearance.texts {
    builder.appearance.layers[text.style.layer].count += 1;
  }
  for fill in builder
    .appearance
    .fills
    .iter()
    .chain(&builder.appearance.masks)
  {
    builder.appearance.layers[fill.style.layer].count += 1;
  }
  for hatch in &builder.appearance.pattern_hatches {
    builder.appearance.layers[hatch.overview.style.layer].count += 1;
  }
  builder.appearance.snap_index = Arc::new(crate::spatial::SpatialIndex::new(
    builder.geometry.len(),
    builder
      .geometry
      .iter()
      .enumerate()
      .filter_map(|(index, primitive)| {
        let mut bounds = Bounds::empty();
        match primitive {
          Primitive::Point(point) => bounds.include(*point),
          Primitive::Path { curves, .. } => {
            for curve in curves {
              match curve {
                crate::geometry::MeasureCurve::Line { start, end } => {
                  bounds.include(*start);
                  bounds.include(*end);
                }
                crate::geometry::MeasureCurve::Round(round) => {
                  // Центр дуги тоже является привязкой, даже вне её видимого габарита.
                  bounds.include_bounds(crate::spatial::neighborhood(round.center, round.radius));
                }
                crate::geometry::MeasureCurve::Polyline { points, .. } => {
                  for point in points {
                    bounds.include(*point);
                  }
                }
              }
            }
          }
        }
        bounds.is_valid().then_some((index, bounds))
      }),
  ));
  builder.appearance.render_index = Arc::new(crate::spatial::SpatialIndex::new(
    builder.geometry.len(),
    builder
      .appearance
      .primitive_bounds
      .iter()
      .copied()
      .enumerate(),
  ));
  builder.appearance.display_geometry = Arc::new(crate::display_geometry::DisplayGeometry::new(
    &builder.geometry,
  ));
  (builder.geometry, builder.appearance, builder.unsupported)
}

struct Builder<'a> {
  elevation_transform: Transform3,
  block_path: String,
  drawing: &'a Drawing,
  raw: &'a RawDxf,
  mtexts: HashMap<usize, &'a Record>,
  wipeouts: HashMap<usize, &'a Record>,
  source_kinds: HashMap<u64, &'a str>,
  unsupported_types: HashMap<String, usize>,
  geometry: Vec<Primitive>,
  appearance: Appearance,
  unsupported: usize,
  layer_ids: HashMap<String, usize>,
}

impl Builder<'_> {
  fn contents(
    &mut self,
    name: &str,
    entities: Vec<&Entity>,
    transform: Transform2,
    parent: Option<&EntityStyle>,
    depth: usize,
    diagnostic: bool,
  ) {
    let extras = self.raw.extras.get(name).map_or(&[][..], Vec::as_slice);
    let handles: HashMap<_, _> = entities
      .iter()
      .enumerate()
      .filter(|(_, e)| e.common.handle.0 != 0)
      .map(|(i, e)| (e.common.handle.0, i))
      .collect();
    let mut used = vec![false; entities.len()];
    let mut used_extras = vec![false; extras.len()];
    let extra_handles: HashMap<_, _> = extras
      .iter()
      .enumerate()
      .filter_map(|(i, r)| {
        r.text(5)
          .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
          .filter(|h| *h != 0)
          .map(|h| (h, i))
      })
      .collect();
    let mut extra_cursor = 0;
    let mut order = Vec::new();
    let mut cursor = 0;
    if let Some(sequence) = self.raw.sequence.get(name) {
      for (kind, handle) in sequence {
        if matches!(kind.as_str(), "HATCH" | "ARC_DIMENSION") {
          let index = if *handle != 0 {
            extra_handles.get(handle).copied()
          } else {
            while extra_cursor < used_extras.len() && used_extras[extra_cursor] {
              extra_cursor += 1;
            }
            (extra_cursor < extras.len()).then_some(extra_cursor)
          };
          if let Some(index) = index
            && !used_extras[index]
          {
            used_extras[index] = true;
            order.push((true, index, *handle));
          }
        } else {
          let index = if *handle != 0 {
            handles.get(handle).copied()
          } else {
            while cursor < used.len() && used[cursor] {
              cursor += 1;
            }
            (cursor < used.len()
              && (entity_kind(&entities[cursor].specific) == kind
                || (kind == "3DLINE" && matches!(entities[cursor].specific, EntityType::Line(_)))))
            .then_some(cursor)
          };
          if let Some(index) = index
            && !used[index]
          {
            used[index] = true;
            order.push((false, index, *handle));
          }
        }
      }
    }
    order.extend(
      entities
        .iter()
        .enumerate()
        .filter(|(i, _)| !used[*i])
        .map(|(i, e)| (false, i, e.common.handle.0)),
    );
    order.extend(
      extras
        .iter()
        .enumerate()
        .filter(|(i, _)| !used_extras[*i])
        .map(|(i, r)| {
          (
            true,
            i,
            r.text(5)
              .and_then(|s| u64::from_str_radix(s.trim(), 16).ok())
              .unwrap_or(0),
          )
        }),
    );
    if order.iter().all(|(_, _, h)| *h != 0)
      || order
        .iter()
        .any(|(_, _, h)| self.raw.draw_order.contains_key(h))
    {
      // Нулевой sort handle означает перенос в конец, как в CAD-порядке отрисовки.
      order.sort_by_key(|(_, _, handle)| {
        match self.raw.draw_order.get(handle).copied().unwrap_or(*handle) {
          0 => u64::MAX,
          value => value,
        }
      });
    }
    for (extra, index, _) in order {
      if extra {
        self.extra(&extras[index], transform, parent, depth, diagnostic);
      } else {
        self.entity(entities[index], transform, parent, depth, diagnostic);
      }
    }
  }

  fn layer(&mut self, name: &str) -> usize {
    if let Some(index) = self.layer_ids.get(name) {
      return *index;
    }
    let source = self.drawing.layers().find(|layer| layer.name == name);
    let raw = self.raw.layers.get(name);
    let mut color = indexed_color(source.and_then(|layer| layer.color.index()).unwrap_or(7));
    let flags = raw.map_or(0, |layer| layer.integer(70, 0));
    let visible = flags & 1 == 0
      && source.is_none_or(|layer| layer.is_layer_on)
      && raw.is_none_or(|layer| layer.integer(62, 7) >= 0);
    if let Some(raw) = raw {
      color = indexed_color(raw.integer(62, 7).unsigned_abs().min(255) as u8);
      if raw.text(420).is_some() {
        color = rgb(raw.integer(420, 0));
      }
    }
    let index = self.appearance.layers.len();
    self.appearance.layers.push(Layer {
      name: name.to_owned(),
      color,
      visible,
      initial_visible: visible,
      locked: flags & 4 != 0,
      count: 0,
    });
    self.layer_ids.insert(name.to_owned(), index);
    index
  }

  fn style(
    &mut self,
    common: &EntityCommon,
    parent: Option<&EntityStyle>,
    diagnostic: bool,
  ) -> EntityStyle {
    let layer = if common.layer == "0" {
      parent
        .map(|style| style.layer)
        .unwrap_or_else(|| self.layer("0"))
    } else {
      self.layer(&common.layer)
    };
    let mut color = if common.color.is_by_block() {
      parent.map_or(self.appearance.layers[layer].color, |style| style.color)
    } else if let Some(index) = common.color.index() {
      indexed_color(index)
    } else {
      self.appearance.layers[layer].color
    };
    if common.color_24_bit != 0 {
      color = rgb(common.color_24_bit);
    }
    let mut transparency = common.transparency;
    if let Some(raw) = self.raw.entity_overrides.get(&common.handle.0) {
      if raw.text(420).is_some() {
        color = rgb(raw.integer(420, 0));
      }
      transparency = raw.integer(440, transparency);
    }
    if transparency & 0x02000000 != 0 {
      color = Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        // В DXF младший байт — непрозрачность: 255 означает полностью видимый объект.
        (transparency & 255) as u8,
      );
    }
    let layer_name = &self.appearance.layers[layer].name;
    let source = self
      .drawing
      .layers()
      .find(|entry| entry.name == *layer_name);
    let type_name = if common.line_type_name.eq_ignore_ascii_case("BYLAYER") {
      source.map_or("CONTINUOUS", |entry| entry.line_type_name.as_str())
    } else {
      common.line_type_name.as_str()
    };
    let pattern: Arc<[f64]> = if type_name.eq_ignore_ascii_case("BYBLOCK") {
      parent.map_or_else(|| Arc::from([]), |style| Arc::clone(&style.pattern))
    } else {
      self
        .drawing
        .line_types()
        .find(|line_type| line_type.name == type_name)
        .map_or_else(
          || Arc::from([]),
          |line_type| {
            line_type
              .dash_dot_space_lengths
              .iter()
              .map(|length| length * common.line_type_scale * self.drawing.header.line_type_scale)
              .collect::<Vec<_>>()
              .into()
          },
        )
    };
    let weight = match common.lineweight_enum_value {
      -1 => source.map_or(0.0, |layer| {
        layer.line_weight.raw_value().max(0) as f32 / 100.0
      }),
      -2 => parent.map_or(0.0, |style| style.line_weight),
      value => value.max(0) as f32 / 100.0,
    };
    let mut parent_layers = parent.map_or_else(Vec::new, |style| style.parent_layers.to_vec());
    if let Some(parent) = parent
      && !parent_layers.contains(&parent.layer)
    {
      parent_layers.push(parent.layer);
    }
    EntityStyle {
      layer,
      parent_layers: parent_layers.into(),
      color,
      visible: common.is_visible && parent.is_none_or(|style| style.visible),
      line_weight: weight,
      pattern,
      diagnostic,
    }
  }

  fn entity(
    &mut self,
    entity: &Entity,
    transform: Transform2,
    parent: Option<&EntityStyle>,
    depth: usize,
    diagnostic: bool,
  ) {
    if depth > 12 {
      self.warning("Превышена глубина вложенности блоков");
      return;
    }
    let style = self.style(&entity.common, parent, diagnostic);
    let elevation_style = style.clone();
    let starts = (
      self.geometry.len(),
      self.appearance.texts.len(),
      self.appearance.fills.len(),
      self.appearance.pattern_hatches.len(),
    );
    match &entity.specific {
      EntityType::Insert(insert) => {
        if let Some(block) = self
          .drawing
          .blocks()
          .find(|block| block.name == insert.name)
        {
          let rows = insert.row_count.max(1) as usize;
          let columns = insert.column_count.max(1) as usize;
          if rows * columns > 100_000 {
            self.warning("Слишком много повторений блока");
            return;
          }
          for row in 0..rows {
            for column in 0..columns {
              let shift = Transform2::insert(
                Point::new(insert.location.x, insert.location.y),
                Point::default(),
                1.0,
                1.0,
                insert.rotation,
              )
              .apply(Point::new(
                column as f64 * insert.column_spacing,
                row as f64 * insert.row_spacing,
              ));
              let local = Transform2::insert(
                shift,
                Point::new(block.base_point.x, block.base_point.y),
                insert.x_scale_factor,
                insert.y_scale_factor,
                insert.rotation,
              );
              let combined = transform
                .then(Transform2::ocs(
                  &insert.extrusion_direction,
                  insert.location.z,
                ))
                .then(local);
              let previous_transform = self.elevation_transform;
              self.elevation_transform =
                previous_transform.then(Transform3::insert(insert, &block.base_point, row, column));
              let previous_path = self.block_path.clone();
              self.block_path = format!(
                "{previous_path}/{}#{:X}[{},{}]",
                insert.name,
                entity.common.handle.0,
                row + 1,
                column + 1
              );
              self.block(&insert.name, combined, &style, depth + 1, diagnostic);
              self.elevation_transform = previous_transform;
              self.block_path = previous_path;
            }
          }
          for attribute in insert.attributes() {
            let mut common = entity.common.clone();
            common.layer = self.appearance.layers[style.layer].name.clone();
            self.entity(
              &Entity {
                common,
                specific: EntityType::Attribute(attribute.clone()),
              },
              transform,
              parent,
              depth + 1,
              false,
            );
          }
        } else {
          self.warning(&format!("Отсутствует блок {}", insert.name));
        }
      }
      EntityType::MText(text) => {
        let content = text.extended_text.join("") + &text.text;
        let raw_text = self
          .mtexts
          .get(&(entity as *const Entity as usize))
          .copied()
          .or_else(|| self.raw.entity_overrides.get(&entity.common.handle.0));
        let raw_direction = raw_text
          .and_then(|raw| {
            raw
              .pairs
              .iter()
              .rev()
              .find(|pair| matches!(pair.0, 11 | 50))
          })
          .map(|pair| pair.0 == 11);
        let use_direction = raw_direction.unwrap_or(text.rotation_angle == 0.0);
        let angle =
          if use_direction && text.x_axis_direction.x.hypot(text.x_axis_direction.y) > 1.0e-12 {
            text.x_axis_direction.y.atan2(text.x_axis_direction.x)
          } else {
            text.rotation_angle.to_radians()
          };
        let attachment = (text.attachment_point as usize).saturating_sub(1).min(8);
        self.text(
          content,
          Point::new(text.insertion_point.x, text.insertion_point.y),
          text.initial_text_height,
          raw_text
            .map_or(text.reference_rectangle_width, |raw| raw.number(41, 0.0))
            .max(0.0),
          1.0,
          angle,
          [
            ((attachment % 3) as f64) * 0.5,
            ((attachment / 3) as f64) * 0.5,
          ],
          text.line_spacing_factor as f32,
          &text.text_style_name,
          text.text_style_name.starts_with("DWG_PROXY_BASELINE_"),
          transform,
          style,
        );
      }
      EntityType::Text(text) => {
        let horizontal = text.horizontal_text_justification as usize;
        let vertical = text.vertical_text_justification as usize;
        let location = if horizontal != 0 || vertical != 0 {
          &text.second_alignment_point
        } else {
          &text.location
        };
        self.text(
          text.value.clone(),
          Point::new(location.x, location.y),
          text.text_height,
          0.0,
          text.relative_x_scale_factor,
          text.rotation.to_radians(),
          [
            match horizontal {
              1 | 4 => 0.5,
              2 => 1.0,
              _ => 0.0,
            },
            match vertical {
              3 => 0.0,
              2 => 0.5,
              _ => 1.0,
            },
          ],
          1.0,
          &text.text_style_name,
          vertical == 0 && horizontal != 4,
          transform.then(Transform2::ocs(&text.normal, text.location.z)),
          style,
        );
      }
      EntityType::Attribute(text) => {
        if text.flags & 1 == 0 {
          self.text(
            text.value.clone(),
            Point::new(text.location.x, text.location.y),
            text.text_height,
            0.0,
            text.relative_x_scale_factor,
            text.rotation.to_radians(),
            [0.0, 1.0],
            1.0,
            &text.text_style_name,
            true,
            transform.then(Transform2::ocs(&text.normal, text.location.z)),
            style,
          );
        }
      }
      EntityType::AttributeDefinition(text) => {
        if text.flags & 3 == 2 {
          self.text(
            text.value.clone(),
            Point::new(text.location.x, text.location.y),
            text.text_height,
            0.0,
            text.relative_x_scale_factor,
            text.rotation.to_radians(),
            [0.0, 1.0],
            1.0,
            &text.text_style_name,
            true,
            transform.then(Transform2::ocs(&text.normal, text.location.z)),
            style,
          );
        }
      }
      EntityType::RotatedDimension(dimension) => {
        self.dimension(&dimension.dimension_base, transform, &style, depth)
      }
      EntityType::RadialDimension(dimension) => {
        self.dimension(&dimension.dimension_base, transform, &style, depth)
      }
      EntityType::DiameterDimension(dimension) => {
        self.dimension(&dimension.dimension_base, transform, &style, depth)
      }
      EntityType::AngularThreePointDimension(dimension) => {
        self.dimension(&dimension.dimension_base, transform, &style, depth)
      }
      EntityType::OrdinateDimension(dimension) => {
        self.dimension(&dimension.dimension_base, transform, &style, depth)
      }
      EntityType::Leader(leader) => {
        let mut style = style;
        style.diagnostic = false;
        let points: Vec<_> = leader
          .vertices
          .iter()
          .map(|point| transform.apply(Point::new(point.x, point.y)))
          .collect();
        if points.len() >= 2 {
          if leader.use_arrowheads {
            let tip = points[0];
            let other = points[1];
            let length = (other.x - tip.x).hypot(other.y - tip.y);
            if length > 1.0e-10 {
              let size = leader.text_annotation_height.max(1.0) * 0.6;
              let (dx, dy) = (
                (other.x - tip.x) / length * size,
                (other.y - tip.y) / length * size,
              );
              self.fill(
                vec![
                  tip,
                  Point::new(tip.x + dx - dy * 0.2, tip.y + dy + dx * 0.2),
                  Point::new(tip.x + dx + dy * 0.2, tip.y + dy - dx * 0.2),
                ],
                vec![0, 1, 2],
                style.clone(),
              );
            }
          }
          self
            .appearance
            .draw_steps
            .push(DrawStep::Primitive(self.geometry.len()));
          self.geometry.push(Primitive::Path {
            points,
            closed: false,
            curves: vec![],
          });
          self.appearance.styles.push(style);
        }
      }
      EntityType::Wipeout(mask) => {
        let raw = self
          .wipeouts
          .get(&(entity as *const Entity as usize))
          .copied();
        self.mask(mask, raw, transform, style);
      }
      EntityType::Solid(solid) => {
        let combined = transform.then(Transform2::ocs(
          &solid.extrusion_direction,
          solid.first_corner.z,
        ));
        let vertices = [
          &solid.first_corner,
          &solid.second_corner,
          &solid.fourth_corner,
          &solid.third_corner,
        ]
        .map(|p| combined.apply(Point::new(p.x, p.y)))
        .to_vec();
        self.fill(vertices, vec![0, 1, 2, 0, 2, 3], style);
      }
      EntityType::Trace(trace) => {
        let vertices = [
          &trace.first_corner,
          &trace.second_corner,
          &trace.fourth_corner,
          &trace.third_corner,
        ]
        .map(|p| transform.apply(Point::new(p.x, p.y)))
        .to_vec();
        self.fill(vertices, vec![0, 1, 2, 0, 2, 3], style);
      }
      _ => {
        let start = self.geometry.len();
        let before_unsupported = self.unsupported;
        append_entity(
          self.drawing,
          entity,
          transform,
          depth,
          &mut self.geometry,
          &mut self.unsupported,
        );
        self
          .appearance
          .styles
          .extend((start..self.geometry.len()).map(|_| style.clone()));
        self
          .appearance
          .draw_steps
          .extend((start..self.geometry.len()).map(DrawStep::Primitive));
        if self.unsupported > before_unsupported {
          let kind = self
            .source_kinds
            .get(&entity.common.handle.0)
            .copied()
            .unwrap_or_else(|| entity_kind(&entity.specific));
          *self.unsupported_types.entry(kind.to_owned()).or_default() +=
            self.unsupported - before_unsupported;
        }
      }
    }
    if diagnostic && let Some(kind) = crate::elevation::entity_name(entity) {
      self.record_elevation(
        format!("{kind} #{:X}{}", entity.common.handle.0, self.block_path),
        crate::elevation::entity_range(entity, self.elevation_transform),
        elevation_style,
        starts,
      );
    }
  }

  fn record_elevation(
    &mut self,
    source: String,
    range: Option<ZRange>,
    style: EntityStyle,
    starts: (usize, usize, usize, usize),
  ) {
    let mut bounds = Bounds::empty();
    for primitive in &self.geometry[starts.0..] {
      if let Some(value) = primitive.bounds() {
        bounds.include_bounds(value);
      }
    }
    for text in &self.appearance.texts[starts.1..] {
      bounds.include_bounds(text.bounds);
    }
    for fill in &self.appearance.fills[starts.2..] {
      bounds.include_bounds(fill.bounds);
    }
    for hatch in &self.appearance.pattern_hatches[starts.3..] {
      bounds.include_bounds(hatch.overview.bounds);
    }
    if bounds.is_valid() {
      self.appearance.elevations.push(ElevationObject {
        source,
        range,
        style,
        bounds,
        primitives: starts.0..self.geometry.len(),
      });
    }
  }

  fn block(
    &mut self,
    name: &str,
    transform: Transform2,
    style: &EntityStyle,
    depth: usize,
    diagnostic: bool,
  ) {
    if depth > 12 {
      self.warning("Превышена глубина вложенности блоков");
      return;
    }
    if let Some(block) = self.drawing.blocks().find(|block| block.name == name) {
      self.contents(
        name,
        block.entities.iter().collect(),
        transform,
        Some(style),
        depth,
        diagnostic,
      );
    } else {
      self.warning(&format!("Отсутствует блок {}", name));
    }
  }

  fn dimension(
    &mut self,
    dimension: &dxf::entities::DimensionBase,
    transform: Transform2,
    style: &EntityStyle,
    depth: usize,
  ) {
    // Сохранённый блок размера содержит точную графику, стрелки и оформленный текст CAD.
    if dimension.block_name.is_empty() {
      self.warning("Размер не содержит графического блока");
    } else {
      self.block(&dimension.block_name, transform, style, depth + 1, false);
    }
  }

  #[allow(clippy::too_many_arguments)]
  fn text(
    &mut self,
    text: String,
    location: Point,
    height: f64,
    width: f64,
    width_factor: f64,
    angle: f64,
    alignment: [f64; 2],
    spacing: f32,
    text_style: &str,
    baseline: bool,
    transform: Transform2,
    style: EntityStyle,
  ) {
    if text.is_empty() {
      return;
    }
    let height = height.abs().max(0.01);
    let font_file = self
      .drawing
      .styles()
      .find(|style| style.name.eq_ignore_ascii_case(text_style))
      .map(|style| style.primary_font_file_name.as_str())
      .unwrap_or(text_style);
    let font = crate::cad_text::fonts::CadFont::from_name(font_file)
      .or_else(|| crate::cad_text::fonts::CadFont::from_name(text_style));
    if font.is_none() {
      let warning = format!(
        "Шрифт «{font_file}» (стиль «{text_style}») заменён на резервный; форма и ширина букв могут отличаться от CAD. SHX пока не поддерживается."
      );
      if !self.appearance.warnings.contains(&warning) {
        self.appearance.warnings.push(warning);
      }
    }
    let origin = transform.apply(location);
    let (sin, cos) = angle.sin_cos();
    let x = transform.apply(Point::new(location.x + cos, location.y + sin));
    let y = transform.apply(Point::new(location.x - sin, location.y + cos));
    let x_axis = Point::new(x.x - origin.x, x.y - origin.y);
    let y_axis = Point::new(y.x - origin.x, y.y - origin.y);
    let plain = crate::cad_text::plain(&text);
    let estimated_width = if width > 0.0 {
      width
    } else {
      plain
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(1) as f64
        * height
        * 0.8
        * width_factor.abs()
    };
    let line_count = plain.lines().count().max(1) as f64;
    let estimated_height = height * 1.8 * line_count;
    let corners = [
      (0.0, 0.0),
      (estimated_width, 0.0),
      (estimated_width, estimated_height),
      (0.0, estimated_height),
    ]
    .map(|(x, y)| {
      let x = x - estimated_width * alignment[0];
      let y = y - estimated_height * alignment[1];
      Point::new(
        origin.x + x * x_axis.x - y * y_axis.x,
        origin.y + x * x_axis.y - y * y_axis.y,
      )
    });
    if let Some(bounds) = Bounds::from_points(corners) {
      self
        .appearance
        .draw_steps
        .push(DrawStep::Text(self.appearance.texts.len()));
      self.appearance.texts.push(CadText {
        text,
        origin,
        x_axis,
        y_axis,
        height,
        width,
        width_factor,
        alignment,
        line_spacing: spacing.clamp(0.5, 3.0),
        font: font.unwrap_or_default(),
        baseline,
        style,
        bounds,
      });
    }
  }

  fn mask(
    &mut self,
    mask: &dxf::entities::Wipeout,
    raw: Option<&Record>,
    _transform: Transform2,
    style: EntityStyle,
  ) {
    if mask.display_options_flags & 1 == 0 {
      return;
    }
    let result = decode_mask(mask, raw, self.elevation_transform);
    match result {
      Ok((vertices, indices)) => {
        if let Some(bounds) = Bounds::from_points(vertices.iter().copied()) {
          self
            .appearance
            .draw_steps
            .push(DrawStep::Mask(self.appearance.masks.len()));
          self.appearance.masks.push(CadFill {
            vertices,
            indices,
            style,
            bounds,
          });
        }
      }
      Err(error) => self.warning(&error),
    }
  }

  fn fill(&mut self, vertices: Vec<Point>, indices: Vec<u32>, style: EntityStyle) {
    if let Some(bounds) = Bounds::from_points(vertices.iter().copied()) {
      self
        .appearance
        .draw_steps
        .push(DrawStep::Fill(self.appearance.fills.len()));
      self.appearance.fills.push(CadFill {
        vertices,
        indices,
        style,
        bounds,
      });
    }
  }

  fn extra(
    &mut self,
    record: &Record,
    transform: Transform2,
    parent: Option<&EntityStyle>,
    depth: usize,
    diagnostic: bool,
  ) {
    let mut common = EntityCommon {
      layer: record.text(8).unwrap_or("0").to_owned(),
      is_visible: record.integer(60, 0) == 0,
      ..Default::default()
    };
    let index = record.integer(62, 256);
    common.color = match index {
      0 => dxf::Color::by_block(),
      1..=255 => dxf::Color::from_index(index as u8),
      _ => dxf::Color::by_layer(),
    };
    common.color_24_bit = record.integer(420, 0);
    common.transparency = record.integer(440, 0);
    let mut style = self.style(&common, parent, false);
    if record.text(420).is_some() && record.integer(420, 0) == 0 {
      style.color = Color32::from_rgba_unmultiplied(0, 0, 0, style.color.a());
    }
    if record.kind == "ARC_DIMENSION" {
      if let Some(name) = record.text(2) {
        self.block(name, transform, &style, depth + 1, false);
      } else {
        self.warning("Дуговой размер не содержит графического блока");
      }
      return;
    }
    let result = crate::hatch::decode(record);
    match result {
      Ok(hatch) => {
        let starts = (
          self.geometry.len(),
          self.appearance.texts.len(),
          self.appearance.fills.len(),
          self.appearance.pattern_hatches.len(),
        );
        let elevation_style = style.clone();
        let elevation_transform =
          self
            .elevation_transform
            .then(Transform3::ocs(&dxf::Vector::new(
              record.number(210, 0.0),
              record.number(220, 0.0),
              record.number(230, 1.0),
            )));
        let mut elevation = ZRange::values(
          hatch
            .loops
            .iter()
            .flatten()
            .map(|p| elevation_transform.z(&dxf::Point::new(p.x, p.y, record.number(30, 0.0)))),
        );
        if let Some(range) = &mut elevation {
          range.approximate = range.max - range.min > 1e-9;
        }
        let combined = transform.then(Transform2::ocs(
          &dxf::Vector::new(
            record.number(210, 0.0),
            record.number(220, 0.0),
            record.number(230, 1.0),
          ),
          record.number(30, 0.0),
        ));
        if hatch.solid {
          match hatch.triangulate() {
            Ok((points, indices)) => self.fill(
              points
                .into_iter()
                .map(|point| combined.apply(point))
                .collect(),
              indices,
              style,
            ),
            Err(error) => self.warning(&error),
          }
        } else {
          match hatch.lines() {
            Ok(lines) => {
              for line in lines {
                self
                  .appearance
                  .draw_steps
                  .push(DrawStep::Primitive(self.geometry.len()));
                self.geometry.push(if line[0] == line[1] {
                  Primitive::Point(combined.apply(line[0]))
                } else {
                  Primitive::Path {
                    points: line.map(|point| combined.apply(point)).to_vec(),
                    closed: false,
                    curves: vec![],
                  }
                });
                self.appearance.styles.push(style.clone());
              }
            }
            Err(error) if crate::hatch::Hatch::exceeds_eager_limit(&error) => {
              self.pattern_hatch(&hatch, combined, style);
            }
            Err(error) => self.warning(&error),
          }
        }
        if diagnostic {
          self.record_elevation(
            format!(
              "HATCH #{}{}",
              record.text(5).unwrap_or("?"),
              self.block_path
            ),
            elevation,
            elevation_style,
            starts,
          );
        }
      }
      Err(error) => self.warning(&error),
    }
  }

  fn pattern_hatch(
    &mut self,
    hatch: &crate::hatch::Hatch,
    transform: Transform2,
    style: EntityStyle,
  ) {
    let result = hatch.prepare_view().and_then(|geometry| {
      let (points, indices) = hatch.triangulate()?;
      let vertices: Vec<_> = points.into_iter().map(|p| transform.apply(p)).collect();
      if indices.is_empty()
        || vertices
          .iter()
          .any(|p| !p.x.is_finite() || !p.y.is_finite())
      {
        return Err("Не удалось построить обзорную границу плотной штриховки".to_owned());
      }
      let bounds =
        Bounds::from_points(vertices.iter().copied()).ok_or("Пустая граница плотной штриховки")?;
      Ok(CadPatternHatch {
        geometry: Arc::new(geometry),
        transform,
        overview: CadFill {
          vertices,
          indices,
          style,
          bounds,
        },
      })
    });
    match result {
      Ok(hatch) => {
        self
          .appearance
          .draw_steps
          .push(DrawStep::Hatch(self.appearance.pattern_hatches.len()));
        self.appearance.pattern_hatches.push(hatch);
        let message = "Плотные штриховки сохранены: в обзорном масштабе отображается их усреднённая плотность с отверстиями; при увеличении — исходные штрихи. Обзорный режим отмечается на холсте.";
        if !self.appearance.warnings.iter().any(|w| w == message) {
          self.appearance.warnings.push(message.to_owned());
        }
      }
      Err(error) => self.warning(&error),
    }
  }

  fn warning(&mut self, text: &str) {
    self.unsupported += 1;
    if !self
      .appearance
      .warnings
      .iter()
      .any(|warning| warning == text)
    {
      self.appearance.warnings.push(text.to_owned());
    }
  }
}

// Библиотека не публикует имя типа; сопоставление нужно для DXF без идентификаторов.
fn entity_kind(entity: &EntityType) -> &'static str {
  match entity {
    EntityType::Face3D(_) => "3DFACE",
    EntityType::Solid3D(_) => "3DSOLID",
    EntityType::ProxyEntity(_) => "ACAD_PROXY_ENTITY",
    EntityType::Arc(_) => "ARC",
    EntityType::ArcAlignedText(_) => "ARCALIGNEDTEXT",
    EntityType::AttributeDefinition(_) => "ATTDEF",
    EntityType::Attribute(_) => "ATTRIB",
    EntityType::Body(_) => "BODY",
    EntityType::Circle(_) => "CIRCLE",
    EntityType::RotatedDimension(_) => "DIMENSION",
    EntityType::RadialDimension(_) => "DIMENSION",
    EntityType::DiameterDimension(_) => "DIMENSION",
    EntityType::AngularThreePointDimension(_) => "DIMENSION",
    EntityType::OrdinateDimension(_) => "DIMENSION",
    EntityType::Ellipse(_) => "ELLIPSE",
    EntityType::Helix(_) => "HELIX",
    EntityType::Image(_) => "IMAGE",
    EntityType::Insert(_) => "INSERT",
    EntityType::Leader(_) => "LEADER",
    EntityType::Light(_) => "LIGHT",
    EntityType::Line(_) => "LINE",
    EntityType::LwPolyline(_) => "LWPOLYLINE",
    EntityType::MLine(_) => "MLINE",
    EntityType::MText(_) => "MTEXT",
    EntityType::OleFrame(_) => "OLEFRAME",
    EntityType::Ole2Frame(_) => "OLE2FRAME",
    EntityType::ModelPoint(_) => "POINT",
    EntityType::Polyline(_) => "POLYLINE",
    EntityType::Ray(_) => "RAY",
    EntityType::Region(_) => "REGION",
    EntityType::RText(_) => "RTEXT",
    EntityType::Section(_) => "SECTION",
    EntityType::Seqend(_) => "SEQEND",
    EntityType::Shape(_) => "SHAPE",
    EntityType::Solid(_) => "SOLID",
    EntityType::Spline(_) => "SPLINE",
    EntityType::Text(_) => "TEXT",
    EntityType::Tolerance(_) => "TOLERANCE",
    EntityType::Trace(_) => "TRACE",
    EntityType::DgnUnderlay(_) => "DGNUNDERLAY",
    EntityType::DwfUnderlay(_) => "DWFUNDERLAY",
    EntityType::PdfUnderlay(_) => "PDFUNDERLAY",
    EntityType::Vertex(_) => "VERTEX",
    EntityType::Wipeout(_) => "WIPEOUT",
    EntityType::XLine(_) => "XLINE",
  }
}

fn decode_mask(
  mask: &dxf::entities::Wipeout,
  raw: Option<&Record>,
  transform: Transform3,
) -> Result<(Vec<Point>, Vec<u32>), String> {
  let fail = || "WIPEOUT: некорректная граница маски".to_owned();
  let width = mask.image_size.x;
  let height = mask.image_size.y;
  if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
    return Err(fail());
  }
  let full = vec![
    Point::new(-0.5, -0.5),
    Point::new(width - 0.5, -0.5),
    Point::new(width - 0.5, height - 0.5),
    Point::new(-0.5, height - 0.5),
  ];
  let clipping = mask.use_clipping && mask.display_options_flags & 4 != 0;
  let mut boundary = if clipping {
    if let Some(raw) = raw {
      // В dxf 0.6.1 неверный код Y для WIPEOUT; читаем реальные группы 14/24.
      let mut points = Vec::new();
      let mut x = None;
      for (code, value) in &raw.pairs {
        if *code == 14 {
          if x.is_some() {
            return Err(fail());
          }
          x = Some(value.trim().parse::<f64>().map_err(|_| fail())?);
        } else if *code == 24 {
          let y = value.trim().parse::<f64>().map_err(|_| fail())?;
          points.push(Point::new(x.take().ok_or_else(fail)?, y));
        }
      }
      if x.is_some() || raw.integer(91, points.len() as i32) != points.len() as i32 {
        return Err(fail());
      }
      points
    } else {
      mask
        .clipping_vertices
        .iter()
        .map(|p| Point::new(p.x, p.y))
        .collect()
    }
  } else {
    full.clone()
  };
  if boundary.len() == 2 {
    let a = boundary[0];
    let b = boundary[1];
    boundary = vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)];
  }
  if boundary.len() > 2 && boundary.first() == boundary.last() {
    boundary.pop();
  }
  if boundary.len() < 3
    || boundary
      .iter()
      .any(|p| !p.x.is_finite() || !p.y.is_finite())
  {
    return Err(fail());
  }
  let inverted = clipping && raw.map_or(mask.is_inside_clipping, |r| r.integer(290, 0) != 0);
  let mut holes = Vec::new();
  let points = if inverted {
    let mut points = full;
    holes.push(points.len());
    points.extend(boundary);
    points
  } else {
    boundary
  };
  let flat: Vec<_> = points.iter().flat_map(|p| [p.x, p.y]).collect();
  let indices = earcutr::earcut(&flat, &holes, 2).map_err(|_| fail())?;
  if indices.is_empty() {
    return Err(fail());
  }
  let vertices = points
    .into_iter()
    .map(|p| {
      // Начало координат клип-границы находится сверху слева, V направлен вверх.
      let u = p.x + 0.5;
      let v = height - p.y - 0.5;
      transform.xy(&dxf::Point::new(
        mask.location.x + mask.u_vector.x * u + mask.v_vector.x * v,
        mask.location.y + mask.u_vector.y * u + mask.v_vector.y * v,
        mask.location.z + mask.u_vector.z * u + mask.v_vector.z * v,
      ))
    })
    .collect::<Vec<_>>();
  if vertices
    .iter()
    .any(|p| !p.x.is_finite() || !p.y.is_finite())
  {
    return Err(fail());
  }
  Ok((vertices, indices.into_iter().map(|i| i as u32).collect()))
}

fn rgb(value: i32) -> Color32 {
  Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn dense_hatch_remains_parametric_with_nested_transforms_and_layer_visibility() {
    let source = concat!(
      "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nDense\n70\n0\n10\n10\n20\n20\n30\n0\n",
      "0\nHATCH\n5\nA1\n8\n0\n70\n0\n91\n2\n",
      "92\n2\n72\n0\n73\n1\n93\n4\n10\n10\n20\n20\n10\n10010\n20\n20\n10\n10010\n20\n10020\n10\n10\n20\n10020\n97\n0\n",
      "92\n2\n72\n0\n73\n1\n93\n4\n10\n2010\n20\n2020\n10\n3010\n20\n2020\n10\n3010\n20\n3020\n10\n2010\n20\n3020\n97\n0\n",
      "75\n0\n78\n1\n53\n0\n43\n0\n44\n0\n45\n0\n46\n1\n79\n2\n49\n1\n49\n-1\n0\nENDBLK\n",
      "0\nBLOCK\n2\nOuter\n70\n0\n10\n0\n20\n0\n30\n0\n0\nINSERT\n5\nA2\n8\nInner\n2\nDense\n10\n50\n20\n60\n41\n2\n42\n3\n50\n90\n0\nENDBLK\n0\nENDSEC\n",
      "0\nSECTION\n2\nENTITIES\n0\nINSERT\n5\nA3\n8\nParent\n2\nOuter\n10\n100\n20\n200\n50\n90\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (primitives, mut appearance, unsupported) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(unsupported, 0);
    assert!(primitives.is_empty() && appearance.fills.is_empty());
    assert_eq!(appearance.draw_steps, [DrawStep::Hatch(0)]);
    assert_eq!(appearance.pattern_hatches.len(), 1);
    let hatch = &appearance.pattern_hatches[0];
    let p = hatch.transform.apply(Point::new(10.0, 20.0));
    assert!((p.x - 40.0).abs() < 1e-8 && (p.y - 250.0).abs() < 1e-8);
    let original = hatch.transform.inverse().unwrap().apply(p);
    assert!((original.x - 10.0).abs() < 1e-8 && (original.y - 20.0).abs() < 1e-8);
    assert!((hatch.overview.bounds.min.x + 19960.0).abs() < 1e-8);
    assert!((hatch.overview.bounds.min.y + 29750.0).abs() < 1e-8);
    assert_eq!(hatch.geometry.source.loops.len(), 2);
    let layer = hatch.overview.style.layer;
    assert_eq!(appearance.layers[layer].name, "Inner");
    assert_eq!(appearance.layers[layer].count, 1);
    assert!(appearance.visible(&hatch.overview.style));
    let parent = appearance
      .layers
      .iter()
      .position(|l| l.name == "Parent")
      .unwrap();
    appearance.layers[parent].visible = false;
    assert!(!appearance.visible(&appearance.pattern_hatches[0].overview.style));
    assert!(
      appearance
        .warnings
        .iter()
        .any(|w| w.contains("Плотные штриховки сохранены"))
    );
    assert!(
      !appearance
        .warnings
        .iter()
        .any(|w| w.contains("Слишком много штрихов"))
    );
  }

  #[test]
  fn wipeout_anonymous_unknown_records_do_not_consume_the_next_typed_entity() {
    let source = concat!(
      "0\nSECTION\n2\nENTITIES\n0\nUNSUPPORTED\n",
      "0\nHATCH\n70\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n10\n20\n0\n10\n0\n20\n10\n97\n0\n75\n0\n76\n1\n98\n0\n",
      "0\nWIPEOUT\n13\n1\n23\n1\n70\n1\n280\n0\n",
      "0\nLINE\n10\n0\n20\n0\n11\n10\n21\n10\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 1);
    assert_eq!(
      appearance.draw_steps,
      [DrawStep::Fill(0), DrawStep::Mask(0), DrawStep::Primitive(0)]
    );
  }

  #[test]
  fn wipeout_default_order_uses_handles_but_anonymous_entities_keep_sequence() {
    for handles in [false, true] {
      let a = if handles { "5\n30\n" } else { "" };
      let b = if handles { "5\n20\n" } else { "" };
      let c = if handles { "5\n10\n" } else { "" };
      let source = format!(
        "0\nSECTION\n2\nENTITIES\n0\nLINE\n{a}10\n30\n20\n0\n11\n35\n21\n0\n0\nWIPEOUT\n{b}13\n1\n23\n1\n70\n1\n280\n0\n0\nLINE\n{c}10\n10\n20\n0\n11\n15\n21\n0\n0\nENDSEC\n0\nEOF\n"
      );
      let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
      let (geometry, appearance, missing) = extract(&drawing, &RawDxf::parse(&source));
      assert_eq!(missing, 0);
      let Primitive::Path { points, .. } = &geometry[0] else {
        panic!()
      };
      assert_eq!(points[0].x, if handles { 10.0 } else { 30.0 });
      assert_eq!(
        appearance.draw_steps,
        [
          DrawStep::Primitive(0),
          DrawStep::Mask(0),
          DrawStep::Primitive(1)
        ]
      );
    }
  }

  #[test]
  fn wipeout_sortentstable_orders_raw_hatch_and_regular_entities_together() {
    let source = concat!(
      "0\nSECTION\n2\nENTITIES\n",
      "0\nHATCH\n5\n10\n70\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n4\n10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n10\n10\n0\n20\n10\n97\n0\n75\n0\n76\n1\n98\n0\n",
      "0\nWIPEOUT\n5\n20\n13\n1\n23\n1\n70\n1\n280\n0\n",
      "0\nLINE\n5\n30\n10\n0\n20\n0\n11\n10\n21\n10\n",
      "0\nENDSEC\n0\nSECTION\n2\nOBJECTS\n0\nSORTENTSTABLE\n5\nA0\n100\nAcDbSortentsTable\n330\nAA\n331\n10\n5\n30\n331\n30\n5\n10\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (geometry, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 0);
    assert_eq!(geometry.len(), 1);
    assert_eq!(
      appearance.draw_steps,
      [DrawStep::Primitive(0), DrawStep::Mask(0), DrawStep::Fill(0)]
    );
  }

  #[test]
  fn wipeout_default_rectangle_and_inverted_boundary_preserve_area() {
    let mask = dxf::entities::Wipeout {
      image_size: dxf::Vector::new(4.0, 2.0, 0.0),
      use_clipping: false,
      display_options_flags: 1,
      ..Default::default()
    };
    let (vertices, indices) = decode_mask(&mask, None, Transform3::IDENTITY).unwrap();
    assert_eq!(
      Bounds::from_points(vertices.iter().copied()).unwrap().max,
      Point::new(4.0, 2.0)
    );
    assert_eq!(indices.len(), 6);
    let mut clipped = mask;
    clipped.use_clipping = true;
    clipped.display_options_flags = 7;
    clipped.is_inside_clipping = true;
    clipped.clipping_vertices = vec![
      dxf::Point::new(0.5, 0.0, 0.0),
      dxf::Point::new(1.5, 0.0, 0.0),
      dxf::Point::new(0.5, 1.0, 0.0),
    ];
    let (vertices, indices) = decode_mask(&clipped, None, Transform3::IDENTITY).unwrap();
    let area: f64 = indices
      .as_chunks::<3>()
      .0
      .iter()
      .map(|triangle| {
        let a = vertices[triangle[0] as usize];
        let b = vertices[triangle[1] as usize];
        let c = vertices[triangle[2] as usize];
        ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
      })
      .sum();
    assert!((area - 7.5).abs() < 1e-8);
  }

  #[test]
  fn wipeout_clipping_is_supported_inside_a_rotated_block() {
    let source = concat!(
      "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nMask\n70\n0\n10\n0\n20\n0\n30\n0\n",
      "0\nWIPEOUT\n5\nA1\n8\n0\n10\n10\n20\n20\n11\n2\n21\n0\n12\n0\n22\n3\n13\n4\n23\n2\n70\n7\n280\n1\n71\n2\n91\n4\n14\n-0.5\n24\n-0.5\n14\n3.5\n24\n-0.5\n14\n-0.5\n24\n1.5\n14\n-0.5\n24\n-0.5\n",
      "0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n8\nMasks\n2\nMask\n10\n100\n20\n200\n50\n90\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 0, "WIPEOUT должен сохраняться как маска");
    assert_eq!(appearance.masks.len(), 1);
    assert_eq!(appearance.draw_steps, [DrawStep::Mask(0)]);
    let mask = &appearance.masks[0];
    assert_eq!(mask.indices.len(), 3);
    assert!((mask.bounds.min.x - 74.0).abs() < 1e-8);
    assert!((mask.bounds.max.x - 80.0).abs() < 1e-8);
    assert!((mask.bounds.min.y - 210.0).abs() < 1e-8);
    assert!((mask.bounds.max.y - 218.0).abs() < 1e-8);
    assert_eq!(appearance.layers[mask.style.layer].name, "Masks");
    let layer = mask.style.layer;
    let mut appearance = appearance;
    appearance.layers[layer].visible = false;
    assert!(!appearance.visible(&appearance.masks[0].style));
  }

  #[test]
  fn explicit_dxf_alpha_does_not_hide_opaque_entities() {
    for alpha in [0_u8, 128, 255] {
      let source = format!(
        "0\nSECTION\n2\nENTITIES\n0\nLINE\n5\nAB\n8\n0\n62\n1\n440\n{}\n10\n0\n20\n0\n11\n10\n21\n0\n0\nENDSEC\n0\nEOF\n",
        0x02000000 | i32::from(alpha)
      );
      let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
      let (_, appearance, missing) = extract(&drawing, &RawDxf::parse(&source));
      assert_eq!(missing, 0);
      assert_eq!(appearance.styles[0].color.a(), alpha);
      if alpha == 255 {
        assert_eq!(appearance.styles[0].color, Color32::RED);
      }
    }
  }

  #[test]
  fn frozen_parent_layer_hides_children_on_other_layers_and_reset_restores_it() {
    let mut drawing = Drawing::new();
    let mut child = Entity::new(EntityType::Circle(dxf::entities::Circle {
      radius: 5.0,
      ..Default::default()
    }));
    child.common.layer = "Child".into();
    drawing.add_block(dxf::Block {
      name: "Block".into(),
      entities: vec![child],
      ..Default::default()
    });
    let mut insert = Entity::new(EntityType::Insert(dxf::entities::Insert {
      name: "Block".into(),
      ..Default::default()
    }));
    insert.common.layer = "Parent".into();
    drawing.add_entity(insert);
    let raw = RawDxf::parse(
      "0\nSECTION\n2\nTABLES\n0\nLAYER\n2\nParent\n70\n1\n62\n7\n0\nENDSEC\n0\nEOF\n",
    );
    let (_, mut appearance, missing) = extract(&drawing, &raw);
    assert_eq!(missing, 0);
    assert!(!appearance.primitive_visible(0));
    for layer in &mut appearance.layers {
      layer.visible = true;
    }
    assert!(appearance.primitive_visible(0));
    appearance.reset_layers();
    assert!(!appearance.primitive_visible(0));
  }

  #[test]
  fn saved_dimension_graphics_include_text_and_do_not_enter_cut_diagnostics() {
    let mut drawing = Drawing::new();
    drawing.add_block(dxf::Block {
      name: "*D1".into(),
      entities: vec![
        Entity::new(EntityType::Line(dxf::entities::Line::new(
          dxf::Point::origin(),
          dxf::Point::new(100.0, 0.0, 0.0),
        ))),
        Entity::new(EntityType::MText(dxf::entities::MText {
          text: "100".into(),
          initial_text_height: 2.5,
          ..Default::default()
        })),
        Entity::new(EntityType::Solid(dxf::entities::Solid {
          first_corner: dxf::Point::new(0.0, 0.0, 0.0),
          second_corner: dxf::Point::new(2.0, 1.0, 0.0),
          third_corner: dxf::Point::new(2.0, -1.0, 0.0),
          fourth_corner: dxf::Point::new(2.0, -1.0, 0.0),
          ..Default::default()
        })),
      ],
      ..Default::default()
    });
    let mut dimension = dxf::entities::RotatedDimension::default();
    dimension.dimension_base.block_name = "*D1".into();
    drawing.add_entity(Entity::new(EntityType::RotatedDimension(dimension)));
    let (geometry, appearance, missing) = extract(&drawing, &RawDxf::default());
    assert_eq!(missing, 0);
    assert_eq!(geometry.len(), 1);
    assert_eq!(appearance.texts[0].text, "100");
    assert_eq!(appearance.fills.len(), 1);
    assert!(!appearance.primitive_diagnostic(0));
  }

  #[test]
  fn ascii_hatch_in_a_rotated_block_preserves_fill_color_and_layer() {
    let source = concat!(
      "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nTile\n70\n0\n10\n0\n20\n0\n30\n0\n",
      "0\nHATCH\n8\n0\n62\n1\n420\n0\n70\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n4\n",
      "10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n5\n10\n0\n20\n5\n97\n0\n75\n0\n76\n1\n98\n0\n",
      "0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n8\nTiles\n2\nTile\n10\n100\n20\n200\n30\n0\n50\n90\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (geometry, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 0);
    assert!(geometry.is_empty());
    assert_eq!(appearance.fills.len(), 1);
    let fill = &appearance.fills[0];
    assert_eq!(fill.style.color, Color32::BLACK);
    assert_eq!(appearance.layers[fill.style.layer].name, "Tiles");
    assert!((fill.bounds.min.x - 95.0).abs() < 1.0e-8);
    assert!((fill.bounds.max.y - 210.0).abs() < 1.0e-8);
    assert_eq!(fill.indices.len(), 6);
  }

  #[test]
  fn mtext_rotation_code_is_not_overridden_by_default_direction() {
    let source = "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n5\nAB\n10\n0\n20\n0\n30\n0\n40\n2\n50\n90\n1\nText\n0\nENDSEC\n0\nEOF\n";
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 0);
    assert!(appearance.texts[0].x_axis.x.abs() < 1.0e-8);
    assert!((appearance.texts[0].x_axis.y - 1.0).abs() < 1.0e-8);
  }

  #[test]
  fn mtext_width_presence_and_degree_angles_survive_import() {
    for handle in ["", "5\nAB\n"] {
      for (width, expected) in [
        ("", 0.0),
        ("41\n0\n", 0.0),
        ("41\n1\n", 1.0),
        ("41\n20\n", 20.0),
      ] {
        for angle in [0.0_f64, 1.0, 45.0, 90.0, -90.0, 270.0] {
          let source = format!(
            "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n{handle}10\n0\n20\n0\n40\n2.5\n{width}50\n{angle}\n1\n11.5\n0\nENDSEC\n0\nEOF\n"
          );
          let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
          let (_, appearance, _) = extract(&drawing, &RawDxf::parse(&source));
          let text = &appearance.texts[0];
          assert_eq!(
            text.width, expected,
            "Ширина: {width:?}, handle: {handle:?}"
          );
          assert!((text.x_axis.x - angle.to_radians().cos()).abs() < 1.0e-8);
          assert!((text.x_axis.y - angle.to_radians().sin()).abs() < 1.0e-8);
        }
      }
    }
  }

  #[test]
  fn mtext_last_direction_or_rotation_group_wins() {
    for (groups, degrees) in [
      ("50\n90\n11\n1\n21\n0\n", 0.0_f64),
      ("11\n1\n21\n0\n50\n45\n", 45.0),
    ] {
      let source = format!(
        "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n5\nAB\n40\n2.5\n{groups}1\n36\n0\nENDSEC\n0\nEOF\n"
      );
      let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
      let (_, appearance, _) = extract(&drawing, &RawDxf::parse(&source));
      assert!((appearance.texts[0].x_axis.x - degrees.to_radians().cos()).abs() < 1.0e-8);
    }
  }

  #[test]
  fn anonymous_mtext_metadata_is_reused_by_rotated_block_inserts() {
    let source = concat!(
      "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nTextBlock\n70\n0\n10\n0\n20\n0\n30\n0\n",
      "0\nMTEXT\n40\n2.5\n50\n45\n1\n36\n",
      "0\nMTEXT\n40\n2.5\n41\n1\n1\nX\n",
      "0\nENDBLK\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n",
      "0\nINSERT\n2\nTextBlock\n10\n0\n20\n0\n",
      "0\nINSERT\n2\nTextBlock\n10\n100\n20\n0\n50\n45\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, _) = extract(&drawing, &RawDxf::parse(source));
    let texts = &appearance.texts;
    assert_eq!(texts.len(), 4);
    assert_eq!(
      texts.iter().map(|text| text.width).collect::<Vec<_>>(),
      vec![0.0, 1.0, 0.0, 1.0]
    );
    assert!((texts[0].x_axis.x - std::f64::consts::FRAC_1_SQRT_2).abs() < 1.0e-8);
    assert!(texts[2].x_axis.x.abs() < 1.0e-8);
    assert!((texts[2].x_axis.y - 1.0).abs() < 1.0e-8);
  }

  #[test]
  fn text_style_resolves_font_file_and_reports_shx_fallback() {
    let source = concat!(
      "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n70\n1\n0\nSTYLE\n2\nDimensions\n3\nOpenSansCondensed-Light.ttf\n0\nENDTAB\n0\nENDSEC\n",
      "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n40\n2.5\n7\nDimensions\n1\n36\n",
      "0\nTEXT\n40\n2\n1\nCaption\n0\nENDSEC\n0\nEOF\n"
    );
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 0);
    assert_eq!(
      appearance.texts[0].font,
      crate::cad_text::fonts::CadFont::CondensedLight
    );
    assert!(
      appearance
        .warnings
        .iter()
        .any(|warning| warning.contains("SHX"))
    );
  }

  #[test]
  fn entity_types_silently_skipped_by_the_library_are_reported() {
    let raw = RawDxf::parse("0\nSECTION\n2\nENTITIES\n0\nMULTILEADER\n5\nAB\n0\nENDSEC\n0\nEOF\n");
    let (_, appearance, missing) = extract(&Drawing::new(), &raw);
    assert_eq!(missing, 1);
    assert!(appearance.warnings[0].contains("MULTILEADER"));
  }
  #[test]
  fn zero_layer_and_byblock_color_follow_the_insert() {
    let mut drawing = Drawing::new();
    drawing.add_layer(dxf::tables::Layer {
      name: "Details".to_owned(),
      color: dxf::Color::from_index(3),
      ..Default::default()
    });
    let mut line = Entity::new(EntityType::Line(dxf::entities::Line::new(
      dxf::Point::origin(),
      dxf::Point::new(10.0, 0.0, 0.0),
    )));
    line.common.color = dxf::Color::by_block();
    drawing.add_block(dxf::Block {
      name: "Part".to_owned(),
      entities: vec![line],
      ..Default::default()
    });
    let mut insert = Entity::new(EntityType::Insert(dxf::entities::Insert {
      name: "Part".to_owned(),
      ..Default::default()
    }));
    insert.common.layer = "Details".to_owned();
    insert.common.color = dxf::Color::from_index(1);
    drawing.add_entity(insert);
    let (_, mut appearance, missing) = extract(&drawing, &RawDxf::default());
    assert_eq!(missing, 0);
    let style = appearance.styles[0].clone();
    assert_eq!(appearance.layers[style.layer].name, "Details");
    assert_eq!(style.color, Color32::RED);
    appearance.layers[style.layer].visible = false;
    assert!(!appearance.primitive_visible(0));
  }
}
