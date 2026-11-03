using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Objects;
using CSMath;

public static class MLineConversion
{
  public static List<Entity> Build(MLine source)
  {
    var style = source.Style ?? throw Failure(source, "Не найден стиль");
    var elements = style.Elements.ToArray();
    if (elements.Length != 2 || source.Vertices.Count < 2)
      throw Failure(source, "Поддерживаются две границы и не менее двух вершин");
    const MLineStyleFlags allowedStyle = MLineStyleFlags.FillOn | MLineStyleFlags.DisplayJoints | MLineStyleFlags.StartSquareCap | MLineStyleFlags.EndSquareCap;
    const MLineFlags allowedEntity = MLineFlags.HasVertices | MLineFlags.Closed | MLineFlags.NoStartCaps | MLineFlags.NoEndCaps;
    if ((style.Flags & ~allowedStyle) != 0 || (source.Flags & ~allowedEntity) != 0)
      throw Failure(source, "Неподдерживаемые флаги или дуговые торцы");
    var normal = Unit(source.Normal, source);
    var points = new List<XYZ[]>();
    foreach (var vertex in source.Vertices)
    {
      if (!Finite(vertex.Position) || !Finite(vertex.Miter) || vertex.Segments.Count != 2)
        throw Failure(source, "Некорректная геометрия или число границ вершины");
      var boundary = new XYZ[2];
      for (var index = 0; index < 2; index++)
      {
        var segment = vertex.Segments[index];
        if (segment.Parameters.Count != 2 || !double.IsFinite(segment.Parameters[0])
          || segment.Parameters[1] != 0 || segment.AreaFillParameters.Count != 0)
          throw Failure(source, "Разрывы границ или заливки пока не поддерживаются");
        // Параметры DWG уже содержат масштаб, выравнивание и окончательное смещение по усу.
        boundary[index] = vertex.Position + vertex.Miter * segment.Parameters[0];
        if (!Finite(boundary[index])) throw Failure(source, "Неконечная координата границы");
      }
      points.Add(boundary);
    }
    var closed = source.Flags.HasFlag(MLineFlags.Closed);
    var count = closed ? points.Count : points.Count - 1;
    var output = new List<Entity>();

    // Заливка каждого участка делится по одной диагонали; общая кромка не рисуется.
    if (style.Flags.HasFlag(MLineStyleFlags.FillOn))
      for (var index = 0; index < count; index++)
      {
        var start = points[index];
        var end = points[(index + 1) % points.Count];
        foreach (var face in FillQuad(start[0], end[0], end[1], start[1], normal, source))
        {
          CopyStyle(source, face);
          face.Color = style.FillColor;
          output.Add(face);
        }
      }

    void AddLine(XYZ a, XYZ b, int elementIndex)
    {
      if (Length(b - a) < 1e-12) return;
      var line = new Line(a, b);
      CopyStyle(source, line);
      line.Color = elements[elementIndex].Color;
      line.LineType = elements[elementIndex].LineType;
      output.Add(line);
    }

    void AddCap(XYZ[] pair)
    {
      if (elements[0].Color.Equals(elements[1].Color) && elements[0].LineType.Name == elements[1].LineType.Name)
        AddLine(pair[0], pair[1], 0);
      else
      {
        var middle = (pair[0] + pair[1]) * 0.5;
        AddLine(pair[0], middle, 0);
        AddLine(pair[1], middle, 1);
      }
    }

    for (var index = 0; index < count; index++)
      for (var boundary = 0; boundary < 2; boundary++)
        AddLine(points[index][boundary], points[(index + 1) % points.Count][boundary], boundary);
    if (style.Flags.HasFlag(MLineStyleFlags.DisplayJoints))
      for (var index = closed ? 0 : 1; index < (closed ? points.Count : points.Count - 1); index++)
        AddCap(points[index]);
    if (!closed)
    {
      if (style.Flags.HasFlag(MLineStyleFlags.StartSquareCap) && !source.Flags.HasFlag(MLineFlags.NoStartCaps)) AddCap(points[0]);
      if (style.Flags.HasFlag(MLineStyleFlags.EndSquareCap) && !source.Flags.HasFlag(MLineFlags.NoEndCaps)) AddCap(points[^1]);
    }
    if (output.Count == 0) throw Failure(source, "Пустая геометрия");
    return output;
  }

  private static IEnumerable<Solid> FillQuad(XYZ a, XYZ b, XYZ c, XYZ d, XYZ normal, MLine source)
  {
    var tolerance = 1e-8 * Math.Max(1, Math.Max(Length(b - a), Length(d - a)));
    if (new[] { b, c, d }.Any(p => Math.Abs(Dot(p - a, normal)) > tolerance))
      throw Failure(source, "Неплоская заливка");
    double Turn(XYZ p, XYZ q, XYZ r) => Dot(XYZ.Cross(q - p, r - p), normal);
    var first = Turn(a, b, c);
    var second = Turn(a, c, d);
    if (first * second >= 0)
    {
      if (Math.Abs(first) > 1e-12) yield return Triangle(a, b, c, normal);
      if (Math.Abs(second) > 1e-12) yield return Triangle(a, c, d, normal);
    }
    else
    {
      first = Turn(a, b, d);
      second = Turn(b, c, d);
      if (first * second < 0) throw Failure(source, "Самопересечение участка заливки");
      if (Math.Abs(first) > 1e-12) yield return Triangle(a, b, d, normal);
      if (Math.Abs(second) > 1e-12) yield return Triangle(b, c, d, normal);
    }
  }

  private static Solid Triangle(XYZ a, XYZ b, XYZ c, XYZ normal)
  {
    // SOLID хранит точки в собственной плоскости; перевод сохраняет мировые XYZ и высоту.
    var x = Math.Abs(normal.X) < 1.0 / 64 && Math.Abs(normal.Y) < 1.0 / 64
      ? XYZ.Cross(XYZ.AxisY, normal) : XYZ.Cross(XYZ.AxisZ, normal);
    x *= 1 / Length(x);
    var y = XYZ.Cross(normal, x);
    XYZ Ocs(XYZ p) => new(Dot(p, x), Dot(p, y), Dot(p, normal));
    return new Solid(Ocs(a), Ocs(b), Ocs(c)) { Normal = normal };
  }

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

  private static bool Finite(XYZ p) => double.IsFinite(p.X) && double.IsFinite(p.Y) && double.IsFinite(p.Z);
  private static double Dot(XYZ a, XYZ b) => a.X * b.X + a.Y * b.Y + a.Z * b.Z;
  private static double Length(XYZ p) => Math.Sqrt(Dot(p, p));
  private static XYZ Unit(XYZ p, MLine source) => Finite(p) && Length(p) > 1e-12 ? p * (1 / Length(p)) : throw Failure(source, "Некорректная нормаль");
  private static NotSupportedException Failure(MLine source, string detail) => new($"MLINE {source.Handle:X}: {detail}.");
}
