using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Entities.AecEntities;
using ACadSharp.Entities.ProxyGraphics;
using ACadSharp.Objects;
using ACadSharp.Tables;
using CSMath;

public sealed record ProxyEntityMapping(ulong SourceHandle, string SourceType, string BlockName, ulong InsertHandle, int GeneratedCount);
public sealed record ProxyConversionReport(int SourceCount, int ReplacedCount, int GeneratedCount, IReadOnlyList<ProxyEntityMapping> Mappings);

public sealed class ProxyEllipseGraphic : IProxyGeometry
{
  public XYZ Center { get; set; }
  public XYZ Normal { get; set; } = XYZ.AxisZ;
  public double MajorRadius { get; set; }
  public double MinorRadius { get; set; }
  public double StartParameter { get; set; }
  public double EndParameter { get; set; }
  public double MajorAngle { get; set; }
  public GraphicsType GraphicsType => (GraphicsType)44;
  public IProxyGeometry Clone() => (IProxyGeometry)MemberwiseClone();
}

public sealed class ProxyRawAttribute51 : IProxyGeometry
{
  public uint RawValue { get; set; }
  public GraphicsType GraphicsType => (GraphicsType)51;
  public IProxyGeometry Clone() => (IProxyGeometry)MemberwiseClone();
}

public static class ProxyConversion
{
  public static ProxyConversionReport Convert(CadDocument doc)
  {
    var source = doc.BlockRecords.SelectMany(b => b.Entities.Select(e => (Block: b, Entity: e))).ToArray();
    var staged = new List<(BlockRecord Owner, Entity Source, BlockRecord Block, Insert Insert)>();
    var names = new HashSet<string>(doc.BlockRecords.Select(b => b.Name), StringComparer.OrdinalIgnoreCase);
    foreach (var item in source)
    {
      if (item.Entity is not (UnknownEntity or Wall or TableEntity or MultiLeader or MLine or Region)) continue;
      if (item.Entity is not (MLine or Region) && !item.Entity.HasProxyGeometries)
        throw Failure(item.Entity, "Нет сохранённой графики для обязательной замены");
      var name = "DWG_PROXY_" + item.Entity.Handle.ToString("X");
      while (!names.Add(name)) name += "_";
      var block = new BlockRecord(name);
      try
      {
        var entities = item.Entity switch {
          MLine multiline => MLineConversion.Build(multiline),
          Region region => ModelerConversion.Build(region),
          _ => Build(doc, item.Entity)
        };
        foreach (var entity in entities) block.Entities.Add(entity);
      }
      catch (Exception ex) when (ex is not OutOfMemoryException)
      {
        throw Failure(item.Entity, ex.Message, ex);
      }
      var insert = new Insert(block);
      CopyStyle(item.Entity, insert);
      staged.Add((item.Block, item.Entity, block, insert));
    }
    // Сначала проверяются все команды всех объектов; ошибка не удаляет ни одного исходника.
    var mappings = new List<ProxyEntityMapping>();
    foreach (var item in staged)
    {
      var handle = item.Source.Handle;
      doc.BlockRecords.Add(item.Block);
      item.Owner.Entities.Add(item.Insert);
      item.Owner.Entities.Remove(item.Source);
      mappings.Add(new(handle, item.Source.GetType().Name, item.Block.Name, item.Insert.Handle, item.Block.Entities.Count));
    }
    return new(source.Length, mappings.Count, mappings.Sum(m => m.GeneratedCount), mappings);
  }

  private static NotSupportedException Failure(Entity entity, string reason, Exception? inner = null) =>
    new($"Сохранённая графика {entity.GetType().Name} handle={entity.Handle:X}: {reason}", inner);

  private static void CopyStyle(Entity source, Entity target)
  {
    target.Layer = source.Layer;
    target.Color = source.Color;
    target.IsInvisible = source.IsInvisible;
    target.LineType = source.LineType;
    target.LineTypeScale = source.LineTypeScale;
    target.LineWeight = source.LineWeight;
    target.Transparency = source.Transparency;
    target.Material = source.Material;
  }

  private static List<Entity> Build(CadDocument doc, Entity source)
  {
    var output = new List<Entity>();
    var style = new Line();
    CopyStyle(source, style);
    var layers = doc.Layers.ToArray();
    var lineTypes = doc.LineTypes.Where(t => !t.Name.Equals("ByLayer", StringComparison.OrdinalIgnoreCase) && !t.Name.Equals("ByBlock", StringComparison.OrdinalIgnoreCase)).ToArray();
    var matrices = new Stack<Matrix4>();
    var matrix = Matrix4.Identity;
    var fill = false;
    void Add(Entity entity)
    {
      CopyStyle(style, entity);
      output.Add(entity);
    }
    foreach (var command in source.ProxyGeometries)
    {
      switch (command)
      {
        case ProxySubentLayer p:
          if (p.LayerIndex < 0 || p.LayerIndex >= layers.Length) throw new NotSupportedException($"Неизвестный индекс слоя {p.LayerIndex}");
          style.Layer = layers[p.LayerIndex];
          break;
        case ProxySubentColor p:
          if (p.ColorIndex < 0 || p.ColorIndex > 256) throw new NotSupportedException($"Некорректный цвет {p.ColorIndex}");
          style.Color = new Color((short)p.ColorIndex);
          break;
        case ProxySubentTrueColor p:
          style.Color = p.ColorMethod switch {
            ProxyColorMethod.ByLayer => Color.ByLayer,
            ProxyColorMethod.ByBlock => Color.ByBlock,
            ProxyColorMethod.ByACI or ProxyColorMethod.ByColor => p.Color,
            ProxyColorMethod.Foreground => new Color(7),
            _ => throw new NotSupportedException($"Неизвестный способ задания цвета {p.ColorMethod}")
          };
          break;
        case ProxyRawAttribute51 p:
          // Тип 51 не описан в публичной спецификации; допускаются только проверенные значения кодировки AcCmTransparency.
          // Внешний отчёт отдельно предупреждает об этой интерпретации и сохраняет число таких команд.
          style.Transparency = p.RawValue switch {
            0 => Transparency.ByLayer,
            0x020000FF => Transparency.Opaque,
            _ => throw new NotSupportedException($"Неизвестное значение атрибута 51: {p.RawValue:X8}")
          };
          break;
        case ProxySubentLineType p:
          if (p.IsByLayer || p.LineTypeIndex == 32767) style.LineType = LineType.ByLayer;
          else if (p.IsByBlock || p.LineTypeIndex == 32766) style.LineType = LineType.ByBlock;
          else if (p.LineTypeIndex < lineTypes.Length) style.LineType = lineTypes[p.LineTypeIndex];
          else throw new NotSupportedException($"Неизвестный тип линии {p.LineTypeIndex}");
          break;
        case ProxySubentLineWeight p: style.LineWeight = p.LineWeight; break;
        case ProxySubentLineTypeScale p: style.LineTypeScale = p.LineTypeScale; break;
        case ProxySubentFillon p: fill = p.IsOn; break;
        case ProxySubEntityMaterial p:
          if (p.MaterialHandle != 0)
          {
            if (doc.GetCadObject(p.MaterialHandle) is not Material material) throw new NotSupportedException($"Неизвестный материал {p.MaterialHandle:X}");
            style.Material = material;
          }
          break;
        // Маркер используется для выбора подобъектов и не меняет видимую геометрию.
        case ProxySubentMarker: break;
        case ProxyPushModelTransform p:
          ValidateMatrix(p.TransformationMatrix);
          matrices.Push(matrix);
          matrix = Multiply(matrix, p.TransformationMatrix);
          break;
        case ProxyPopModelTransform:
          if (matrices.Count == 0) throw new NotSupportedException("Пустой стек преобразований");
          matrix = matrices.Pop();
          break;
        case ProxyPolyline p:
          if (p.Points.Count == 0) throw new NotSupportedException("Полилиния не содержит точек");
          if (p.Points.Count == 1) Add(new ACadSharp.Entities.Point(Point(matrix, p.Points[0])));
          for (var i = 1; i < p.Points.Count; i++) Add(new Line(Point(matrix, p.Points[i - 1]), Point(matrix, p.Points[i])));
          fill = false;
          break;
        case ProxyPolygon p:
          var polygon = p.Points.Select(v => Point(matrix, v)).ToList();
          if (fill)
          {
            foreach (var triangle in Triangulate(polygon)) Add(SolidTriangle(triangle.A, triangle.B, triangle.C));
          }
          else
          {
            if (polygon.Count < 3) throw new NotSupportedException("Полигон содержит меньше трёх точек");
            for (var i = 0; i < polygon.Count; i++) Add(new Line(polygon[i], polygon[(i + 1) % polygon.Count]));
          }
          fill = false;
          break;
        case ProxyCircle p:
          Add(Circular(matrix, p.Center, p.Normal, p.Radius, null, 0));
          fill = false;
          break;
        case ProxyCircularArc p:
          if (p.ArcType != 0) throw new NotSupportedException($"Не поддержан тип дуги {p.ArcType}");
          Add(Circular(matrix, p.Center, p.Normal, p.Radius, p.StartVectorDirection, p.SweepAngle));
          fill = false;
          break;
        case ProxyEllipseGraphic p:
          var (ex, ey, _) = Basis(p.Normal);
          var major = (ex * Math.Cos(p.MajorAngle) + ey * Math.Sin(p.MajorAngle)) * p.MajorRadius;
          var minor = (-ex * Math.Sin(p.MajorAngle) + ey * Math.Cos(p.MajorAngle)) * p.MinorRadius;
          Add(Elliptical(Point(matrix, p.Center), Vector(matrix, major), Vector(matrix, minor), p.StartParameter, p.EndParameter));
          fill = false;
          break;
        case ProxyUnicodeText2 p:
          Add(Text(doc, matrix, p));
          fill = false;
          break;
        case ProxyShell p:
          Shell(doc, p, matrix, Add);
          fill = false;
          break;
        default:
          throw new NotSupportedException($"Не поддержана команда {command.GetType().Name} ({command.GraphicsType})");
      }
    }
    if (matrices.Count != 0) throw new NotSupportedException("Незакрытый стек преобразований");
    return output;
  }

  private static Ellipse Elliptical(XYZ center, XYZ u, XYZ v, double start, double end)
  {
    if (!double.IsFinite(start) || !double.IsFinite(end) || Length(u) < 1e-12 || Length(v) < 1e-12) throw new NotSupportedException("Некорректный эллипс");
    var phi = 0.5 * Math.Atan2(2 * Dot(u, v), Dot(u, u) - Dot(v, v));
    var major = u * Math.Cos(phi) + v * Math.Sin(phi);
    var minor = -u * Math.Sin(phi) + v * Math.Cos(phi);
    var normal = Unit(XYZ.Cross(major, minor));
    var a = Angle(start - phi);
    var span = end - start;
    while (span < 0) span += Math.PI * 2;
    if (span > Math.PI * 2 + 1e-8) throw new NotSupportedException("Некорректный интервал эллиптической дуги");
    return new Ellipse { Center = center, MajorAxisEndPoint = major, Normal = normal, RadiusRatio = Length(minor) / Length(major), StartParameter = a, EndParameter = a + span };
  }

  private static Entity Circular(Matrix4 matrix, XYZ center, XYZ normal, double radius, XYZ? start, double sweep)
  {
    if (!(radius > 0) || !double.IsFinite(radius)) throw new NotSupportedException("Некорректный радиус");
    var (x, y, _) = Basis(normal);
    var u = Vector(matrix, x);
    var v = Vector(matrix, y);
    var lu = Length(u);
    var lv = Length(v);
    if (lu < 1e-12 || lv < 1e-12 || Math.Abs(lu - lv) > 1e-8 * Math.Max(lu, lv) || Math.Abs(Dot(u, v)) > 1e-8 * lu * lv)
      throw new NotSupportedException("Неравномерное преобразование окружности требует эллипса");
    var n = Unit(XYZ.Cross(u, v));
    var (ox, oy, _) = Basis(n);
    var wcs = Point(matrix, center);
    var ocs = new XYZ(Dot(wcs, ox), Dot(wcs, oy), Dot(wcs, n));
    if (start is null) return new Circle { Center = ocs, Normal = n, Radius = radius * lu };
    if (!double.IsFinite(sweep) || Math.Abs(sweep) > Math.PI * 2 + 1e-8) throw new NotSupportedException("Некорректный угол дуги");
    var direction = Vector(matrix, Unit(start.Value));
    var angle = Math.Atan2(Dot(direction, oy), Dot(direction, ox));
    var a = sweep < 0 ? angle + sweep : angle;
    var b = sweep < 0 ? angle : angle + sweep;
    return new Arc { Center = ocs, Normal = n, Radius = radius * lu, StartAngle = Angle(a), EndAngle = Angle(b) };
  }

  private static Entity Text(CadDocument doc, Matrix4 matrix, ProxyUnicodeText2 text)
  {
    if (text.IsVertical) throw new NotSupportedException("Вертикальный текст требует отдельного преобразования");
    var x = Unit(text.TextDirection);
    var y = Unit(XYZ.Cross(Unit(text.Normal), x));
    var tx = Vector(matrix, x);
    var ty = Vector(matrix, y);
    var sx = Length(tx);
    var sy = Length(ty);
    if (sx < 1e-12 || sy < 1e-12 || Math.Abs(Dot(tx, ty)) > 1e-8 * sx * sy) throw new NotSupportedException("Сдвиговая матрица текста не поддержана");
    var normal = Unit(XYZ.Cross(tx, ty));
    var (ox, oy, _) = Basis(normal);
    var wcs = Point(matrix, text.StartPoint);
    var font = text.FontDescriptor;
    var filename = string.IsNullOrEmpty(font.FontFilename) ? font.Typeface : font.FontFilename;
    var style = doc.TextStyles.FirstOrDefault(s => s.Filename == filename && s.BigFontFilename == text.BigFontFilename && s.TrueType == ((font.IsBold ? FontFlags.Bold : FontFlags.Regular) | (font.IsItalic ? FontFlags.Italic : FontFlags.Regular)));
    // Новый стиль присоединится к документу только вместе с готовым блоком.
    style ??= new TextStyle("DWG_PROXY_FONT_" + StableName(filename + "|" + font.IsBold + "|" + font.IsItalic + "|" + text.BigFontFilename)) {
      Filename = filename, BigFontFilename = text.BigFontFilename,
      TrueType = (font.IsBold ? FontFlags.Bold : FontFlags.Regular) | (font.IsItalic ? FontFlags.Italic : FontFlags.Regular)
    };
    if (text.IsUnderlined || text.IsOverlined || Math.Abs(text.TrackingPercentage - 1) > 1e-9)
    {
      if (text.IsBackwards || text.IsUpsideDown) throw new NotSupportedException("Отражённый форматированный текст требует отдельного преобразования");
      // Префикс сохраняет указание на исходную базовую линию для собственного просмотрщика.
      style = new TextStyle("DWG_PROXY_BASELINE_" + StableName(filename + "|" + font.IsBold + "|" + font.IsItalic + "|" + text.BigFontFilename)) {
        Filename = filename, BigFontFilename = text.BigFontFilename,
        TrueType = (font.IsBold ? FontFlags.Bold : FontFlags.Regular) | (font.IsItalic ? FontFlags.Italic : FontFlags.Regular)
      };
      var culture = System.Globalization.CultureInfo.InvariantCulture;
      var value = text.Text.Replace("\\", "\\\\").Replace("{", "\\{").Replace("}", "\\}");
      var format = "\\T" + text.TrackingPercentage.ToString("G17", culture) + ";\\W" + (text.WidthFactor * sx / sy).ToString("G17", culture) + ";\\Q" + (text.ObliqueAngle * 180 / Math.PI).ToString("G17", culture) + ";";
      if (text.IsUnderlined) format += "\\L";
      if (text.IsOverlined) format += "\\O";
      return new MText {
        Value = format + value, InsertPoint = wcs, Normal = normal, AlignmentPoint = tx / sx,
        AttachmentPoint = AttachmentPointType.BottomLeft, Height = text.Height * sy, Style = style
      };
    }
    return new TextEntity {
      Value = text.Text, InsertPoint = new XYZ(Dot(wcs, ox), Dot(wcs, oy), Dot(wcs, normal)),
      Normal = normal, Height = text.Height * sy, WidthFactor = text.WidthFactor * sx / sy,
      ObliqueAngle = text.ObliqueAngle, Rotation = Math.Atan2(Dot(tx, oy), Dot(tx, ox)), Style = style,
      Mirror = (TextMirrorFlag)((text.IsBackwards ? 2 : 0) | (text.IsUpsideDown ? 4 : 0))
    };
  }

  private static string StableName(string value)
  {
    uint hash = 2166136261;
    foreach (var c in value) hash = unchecked((hash ^ c) * 16777619);
    return hash.ToString("X8");
  }

  private static void Shell(CadDocument doc, ProxyShell shell, Matrix4 matrix, Action<Entity> add)
  {
    if (shell.Faces.Count == 0) throw new NotSupportedException("Оболочка без граней");
    var edgeOffset = 0;
    for (var f = 0; f < shell.Faces.Count; f++)
    {
      var indices = shell.Faces[f];
      if (indices.Any(i => i < 0 || i >= shell.Vertices.Count)) throw new NotSupportedException("Некорректный индекс вершины оболочки");
      var vertices = indices.Select(i => Point(matrix, shell.Vertices[i])).ToList();
      var faceTraits = shell.FaceTraits;
      var edgeTraits = shell.EdgeTraits;
      foreach (var triangle in Triangulate(vertices))
      {
        var face = SolidTriangle(triangle.A, triangle.B, triangle.C);
        add(face);
        ApplyTraits(doc, face, faceTraits, f);
      }
      // Грани и их рёбра — части одной оболочки; внутренние диагонали скрыты.
      for (var i = 0; i < indices.Length; i++)
      {
        if (edgeTraits?.VisibilityIndicators.Count > 0 && edgeTraits.VisibilityIndicators[edgeOffset + i] == 0) continue;
        var edge = new Line(vertices[i], vertices[(i + 1) % vertices.Count]);
        add(edge);
        ApplyTraits(doc, edge, faceTraits, f);
        ApplyTraits(doc, edge, edgeTraits, edgeOffset + i);
        if (edgeTraits?.LineTypeHandles.Count > 0)
        {
          var handle = edgeTraits.LineTypeHandles[edgeOffset + i];
          edge.LineType = doc.GetCadObject(handle) as LineType ?? throw new NotSupportedException($"Неизвестный тип линии оболочки {handle:X}");
        }
      }
      edgeOffset += indices.Length;
    }
  }

  private static void ApplyTraits(CadDocument doc, Entity entity, PrimitiveTraits? traits, int index)
  {
    if (traits == null) return;
    if (traits.Colors.Count > 0) entity.Color = new Color(checked((short)traits.Colors[index]));
    if (traits.LayerHandles.Count > 0) entity.Layer = doc.GetCadObject(traits.LayerHandles[index]) as Layer ?? throw new NotSupportedException("Неизвестный слой оболочки");
    if (traits.VisibilityIndicators.Count > 0) entity.IsInvisible |= traits.VisibilityIndicators[index] == 0;
  }

  private static Solid SolidTriangle(XYZ a, XYZ b, XYZ c)
  {
    var (x, y, n) = Basis(XYZ.Cross(b - a, c - a));
    XYZ Ocs(XYZ p) => new(Dot(p, x), Dot(p, y), Dot(p, n));
    return new Solid(Ocs(a), Ocs(b), Ocs(c)) { Normal = n };
  }

  private static IEnumerable<(XYZ A, XYZ B, XYZ C)> Triangulate(List<XYZ> points)
  {
    if (points.Count > 1 && Length(points[0] - points[^1]) < 1e-10) points.RemoveAt(points.Count - 1);
    if (points.Count < 3) throw new NotSupportedException("Грань содержит меньше трёх точек");
    var normal = XYZ.Zero;
    for (var i = 0; i < points.Count; i++) normal += XYZ.Cross(points[i] - points[0], points[(i + 1) % points.Count] - points[0]);
    if (Length(normal) < 1e-12) throw new NotSupportedException("Вырожденная грань");
    var (x, y, n) = Basis(normal);
    var projected = points.Select(p => new XY(Dot(p - points[0], x), Dot(p - points[0], y))).ToArray();
    var size = projected.Max(p => Math.Max(Math.Abs(p.X), Math.Abs(p.Y)));
    var epsilon = Math.Max(1e-12, size * size * 1e-12);
    if (points.Any(p => Math.Abs(Dot(p - points[0], n)) > Math.Max(1e-8, size * 1e-8))) throw new NotSupportedException("Неплоская грань");
    var remaining = Enumerable.Range(0, points.Count).ToList();
    double Cross(int a, int b, int c) => (projected[b].X - projected[a].X) * (projected[c].Y - projected[a].Y) - (projected[b].Y - projected[a].Y) * (projected[c].X - projected[a].X);
    while (remaining.Count > 3)
    {
      var found = false;
      for (var i = 0; i < remaining.Count; i++)
      {
        var a = remaining[(i + remaining.Count - 1) % remaining.Count];
        var b = remaining[i];
        var c = remaining[(i + 1) % remaining.Count];
        var cross = Cross(a, b, c);
        if (Math.Abs(cross) <= epsilon) { remaining.RemoveAt(i); found = true; break; }
        if (cross < 0) continue;
        if (remaining.Any(p => p != a && p != b && p != c && Cross(a, b, p) >= -epsilon && Cross(b, c, p) >= -epsilon && Cross(c, a, p) >= -epsilon)) continue;
        yield return (points[a], points[b], points[c]);
        remaining.RemoveAt(i); found = true; break;
      }
      if (!found) throw new NotSupportedException("Самопересекающаяся грань или неподдержанная топология оболочки");
    }
    if (remaining.Count == 3) yield return (points[remaining[0]], points[remaining[1]], points[remaining[2]]);
  }

  private static double Dot(XYZ a, XYZ b) => a.X * b.X + a.Y * b.Y + a.Z * b.Z;
  private static double Length(XYZ a) => Math.Sqrt(Dot(a, a));
  private static XYZ Unit(XYZ a) => Length(a) > 1e-15 ? a / Length(a) : throw new NotSupportedException("Нулевой вектор направления");
  private static double Angle(double a) => (a % (Math.PI * 2) + Math.PI * 2) % (Math.PI * 2);
  private static (XYZ X, XYZ Y, XYZ N) Basis(XYZ normal)
  {
    var n = Unit(normal);
    var x = Unit(XYZ.Cross(Math.Abs(n.X) < 1.0 / 64 && Math.Abs(n.Y) < 1.0 / 64 ? XYZ.AxisY : XYZ.AxisZ, n));
    return (x, XYZ.Cross(n, x), n);
  }
  private static XYZ Vector(Matrix4 m, XYZ p) => new(m.M00 * p.X + m.M01 * p.Y + m.M02 * p.Z, m.M10 * p.X + m.M11 * p.Y + m.M12 * p.Z, m.M20 * p.X + m.M21 * p.Y + m.M22 * p.Z);
  private static XYZ Point(Matrix4 m, XYZ p) => Vector(m, p) + new XYZ(m.M03, m.M13, m.M23);
  private static Matrix4 Multiply(Matrix4 a, Matrix4 b)
  {
    var result = Matrix4.Zero;
    for (var i = 0; i < 4; i++) for (var j = 0; j < 4; j++) for (var k = 0; k < 4; k++) result[i, j] += a[i, k] * b[k, j];
    return result;
  }
  private static void ValidateMatrix(Matrix4 m)
  {
    for (var i = 0; i < 4; i++) for (var j = 0; j < 4; j++) if (!double.IsFinite(m[i, j])) throw new NotSupportedException("Неконечный элемент матрицы");
    if (Math.Abs(m.M30) > 1e-12 || Math.Abs(m.M31) > 1e-12 || Math.Abs(m.M32) > 1e-12 || Math.Abs(m.M33 - 1) > 1e-12) throw new NotSupportedException("Проективная матрица не поддержана");
  }
}
