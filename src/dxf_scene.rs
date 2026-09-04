use std::{collections::HashMap, sync::Arc};

use dxf::{
  Drawing,
  entities::{Entity, EntityCommon, EntityType},
};
use eframe::egui::Color32;

use crate::{
  cad_scene::{Appearance, CadFill, CadText, EntityStyle, Layer, indexed_color},
  dxf_import::{Transform2, append_entity},
  geometry::{Bounds, Point, Primitive},
  raw_dxf::RawDxf,
};

pub fn extract(drawing: &Drawing, raw: &RawDxf) -> (Vec<Primitive>, Appearance, usize) {
  let mut builder = Builder {
    drawing,
    raw,
    geometry: Vec::new(),
    appearance: Appearance::default(),
    unsupported: 0,
    layer_ids: HashMap::new(),
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
  for entity in drawing.entities() {
    builder.entity(entity, Transform2::IDENTITY, None, 0, true);
  }
  builder.extras("", Transform2::IDENTITY, None, 0);
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
  for fill in &builder.appearance.fills {
    builder.appearance.layers[fill.style.layer].count += 1;
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
  drawing: &'a Drawing,
  raw: &'a RawDxf,
  geometry: Vec<Primitive>,
  appearance: Appearance,
  unsupported: usize,
  layer_ids: HashMap<String, usize>,
}

impl Builder<'_> {
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
              self.block(&insert.name, combined, &style, depth + 1, diagnostic);
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
        let raw_direction = self
          .raw
          .entity_overrides
          .get(&entity.common.handle.0)
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
            text.rotation_angle
          };
        let attachment = (text.attachment_point as usize).saturating_sub(1).min(8);
        self.text(
          content,
          Point::new(text.insertion_point.x, text.insertion_point.y),
          text.initial_text_height,
          text.reference_rectangle_width,
          1.0,
          angle,
          [
            ((attachment % 3) as f64) * 0.5,
            ((attachment / 3) as f64) * 0.5,
          ],
          text.line_spacing_factor as f32,
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
          self.geometry.push(Primitive::Path {
            points,
            closed: false,
            curves: vec![],
          });
          self.appearance.styles.push(style);
        }
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
      }
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
      for child in &block.entities {
        self.entity(child, transform, Some(style), depth, diagnostic);
      }
      self.extras(name, transform, Some(style), depth);
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
    transform: Transform2,
    style: EntityStyle,
  ) {
    if text.is_empty() {
      return;
    }
    let height = height.abs().max(0.01);
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
        style,
        bounds,
      });
    }
  }

  fn fill(&mut self, vertices: Vec<Point>, indices: Vec<u32>, style: EntityStyle) {
    if let Some(bounds) = Bounds::from_points(vertices.iter().copied()) {
      self.appearance.fills.push(CadFill {
        vertices,
        indices,
        style,
        bounds,
      });
    }
  }

  fn extras(
    &mut self,
    block: &str,
    transform: Transform2,
    parent: Option<&EntityStyle>,
    depth: usize,
  ) {
    let Some(records) = self.raw.extras.get(block) else {
      return;
    };
    for record in records {
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
        continue;
      }
      let result = crate::hatch::decode(record);
      match result {
        Ok(hatch) => {
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
              Err(error) => self.warning(&error),
            }
          }
        }
        Err(error) => self.warning(&error),
      }
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

fn rgb(value: i32) -> Color32 {
  Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

#[cfg(test)]
mod tests {
  use super::*;

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
    let source = "0\nSECTION\n2\nENTITIES\n0\nMTEXT\n5\nAB\n10\n0\n20\n0\n30\n0\n40\n2\n50\n1.5707963267948966\n1\nText\n0\nENDSEC\n0\nEOF\n";
    let drawing = Drawing::load(&mut std::io::Cursor::new(source.as_bytes())).unwrap();
    let (_, appearance, missing) = extract(&drawing, &RawDxf::parse(source));
    assert_eq!(missing, 0);
    assert!(appearance.texts[0].x_axis.x.abs() < 1.0e-8);
    assert!((appearance.texts[0].x_axis.y - 1.0).abs() < 1.0e-8);
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
