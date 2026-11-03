using ACadSharp.Entities;
using CSMath;
using System.Text;

public static class ModelerConversion
{
  // Узкий читатель аналитических плоских REGION. Любая неподдержанная геометрия отклоняется целиком.
  private sealed record Token(byte Tag, object Value);
  private sealed record Record(string Name, Token[] Fields);
  private static NotSupportedException Error(string message) => new("REGION SAB: " + message);
  private static double Dot(XYZ a, XYZ b) => a.X * b.X + a.Y * b.Y + a.Z * b.Z;
  private static double Length(XYZ v)
  {
    var squared = Dot(v, v);
    Require(double.IsFinite(squared), "переполнение нормы вектора");
    return Math.Sqrt(squared);
  }
  private static XYZ Finite(XYZ v)
  {
    Require(double.IsFinite(v.X) && double.IsFinite(v.Y) && double.IsFinite(v.Z), "переполнение преобразованных координат");
    return v;
  }
  private static XYZ Unit(XYZ v) => Length(v) > 1e-12 ? v / Length(v) : throw Error("нулевой вектор");
  private static void Require(bool condition, string message) { if (!condition) throw Error(message); }
  private static int Ref(Record r, int index) => (int)r.Fields[index].Value;
  private static double Real(Record r, int index) => (double)r.Fields[index].Value;
  private static XYZ Vector(Record r, int index) => (XYZ)r.Fields[index].Value;
  private static bool Flag(Record r, int index) => (bool)r.Fields[index].Value;

  public static List<Entity> Build(Region region)
  {
    var records = Read(region.AcisData ?? Array.Empty<byte>());
    foreach (var record in records) Validate(record);
    foreach (var record in records)
      foreach (var field in record.Fields.Where(t => t.Tag == 12))
        Require((int)field.Value >= -1 && (int)field.Value < records.Count, "ссылка за пределами таблицы");
    var visited = new HashSet<int>();
    Record Get(int id, string type)
    {
      Require(id >= 0 && id < records.Count && records[id].Name == type, $"ожидалась запись {type}, ссылка {id}");
      visited.Add(id);
      return records[id];
    }
    int Single(string type)
    {
      var ids = Enumerable.Range(0, records.Count).Where(i => records[i].Name == type).ToArray();
      Require(ids.Length == 1, $"ожидалась одна запись {type}");
      return ids[0];
    }
    void Null(Record r, params int[] indices)
    {
      foreach (var index in indices) Require(Ref(r, index) == -1, $"неподдержанная связь {r.Name}[{index}]");
    }
    var bodyId = Single("body"); var body = Get(bodyId, "body"); Null(body, 2, 4);
    var lumpId = Ref(body, 3); var lump = Get(lumpId, "lump"); Null(lump, 2, 3);
    Require(Ref(lump, 5) == bodyId, "неверный владелец lump");
    var shellId = Ref(lump, 4); var shell = Get(shellId, "shell"); Null(shell, 2, 3, 4, 6);
    Require(Ref(shell, 7) == lumpId, "неверный владелец shell");
    var faceId = Ref(shell, 5); var face = Get(faceId, "face"); Null(face, 2, 3, 6);
    Require(Ref(face, 5) == shellId && Flag(face, 9), "неподдержанная грань REGION");
    var plane = Get(Ref(face, 7), "plane-surface"); Null(plane, 2);
    var planeOrigin = Vector(plane, 3); var planeNormal = Unit(Vector(plane, 4));
    Require(Math.Abs(Dot(planeNormal, Unit(Vector(plane, 5)))) < 1e-8, "неортогональный базис плоскости");
    var x = XYZ.AxisX; var y = XYZ.AxisY; var z = XYZ.AxisZ; var translation = XYZ.Zero;
    if (Ref(body, 5) != -1)
    {
      var transform = Get(Ref(body, 5), "transform");
      x = Vector(transform, 2); y = Vector(transform, 3); z = Vector(transform, 4); translation = Vector(transform, 5);
      var scale = Length(x);
      Require(scale > 1e-12 && Math.Abs(Length(y) / scale - 1) < 1e-8 && Math.Abs(Length(z) / scale - 1) < 1e-8, "неравномерный масштаб");
      Require(Math.Abs(Dot(x, y)) / (scale * scale) < 1e-8 && Math.Abs(Dot(x, z)) / (scale * scale) < 1e-8 && Math.Abs(Dot(y, z)) / (scale * scale) < 1e-8, "сдвиговый transform");
      Require(!Flag(transform, 9) && Math.Abs(Math.Abs(Real(transform, 6)) / scale - 1) < 1e-8, "несогласованный transform");
    }
    XYZ Direction(XYZ p) => Finite(x * p.X + y * p.Y + z * p.Z);
    XYZ Position(XYZ p) => Finite(Direction(p) + translation);
    XYZ Vertex(int id)
    {
      var vertex = Get(id, "vertex"); Null(vertex, 2);
      Get(Ref(vertex, 3), "edge");
      var point = Get(Ref(vertex, 5), "point"); Null(point, 2);
      var value = Vector(point, 3);
      Require(Math.Abs(Dot(value - planeOrigin, planeNormal)) <= 1e-6, "вершина вне плоскости REGION");
      return value;
    }
    void Near(XYZ a, XYZ b) => Require(Length(a - b) <= 1e-6, "геометрия кривой не совпадает с вершиной");
    var output = new List<Entity>(); var edges = new HashSet<int>(); var coedges = new HashSet<int>(); var loops = new HashSet<int>();
    var loopId = Ref(face, 4);
    while (loopId != -1)
    {
      Require(loops.Add(loopId), "цикл в списке контуров");
      var loop = Get(loopId, "loop"); Null(loop, 2);
      Require(Ref(loop, 5) == faceId, "неверный владелец loop");
      var first = Ref(loop, 4); var current = first;
      XYZ? firstPoint = null; XYZ? previousEnd = null;
      do
      {
        Require(coedges.Add(current), "повторная coedge или незамкнутый контур");
        var coedge = Get(current, "coedge"); Null(coedge, 2, 5, 10);
        Require(Ref(coedge, 8) == loopId && Ref(coedge, 9) == 0, "неподдержанная coedge");
        var next = Get(Ref(coedge, 3), "coedge"); var previous = Get(Ref(coedge, 4), "coedge");
        Require(Ref(next, 4) == current && Ref(previous, 3) == current, "разорваны связи coedge");
        var edgeId = Ref(coedge, 6); var edge = Get(edgeId, "edge"); Null(edge, 2);
        Require(edges.Add(edgeId) && Ref(edge, 7) == current, "повторное либо несогласованное ребро REGION");
        Require(!Flag(edge, 9), "обратная параметризация edge пока не поддерживается");
        var start = Vertex(Ref(edge, 3)); var end = Vertex(Ref(edge, 5));
        var directedStart = Flag(coedge, 7) ? end : start; var directedEnd = Flag(coedge, 7) ? start : end;
        if (previousEnd is XYZ previousPoint) Near(previousPoint, directedStart); else firstPoint = directedStart;
        previousEnd = directedEnd;
        var curveId = Ref(edge, 8); Require(curveId >= 0, "у ребра нет кривой");
        var curve = records[curveId]; visited.Add(curveId); Null(curve, 2);
        var a = Real(edge, 4); var b = Real(edge, 6);
        Entity entity;
        if (curve.Name == "straight-curve")
        {
          var origin = Vector(curve, 3); var direction = Vector(curve, 4);
          Require(Length(direction) > 1e-12 && Math.Abs(b - a) > 1e-12, "нулевое прямое ребро");
          Near(origin + direction * a, start); Near(origin + direction * b, end);
          entity = new Line { StartPoint = Position(start), EndPoint = Position(end) };
        }
        else if (curve.Name == "ellipse-curve")
        {
          var center = Vector(curve, 3); var normal = Unit(Vector(curve, 4)); var major = Vector(curve, 5); var ratio = Real(curve, 6);
          Require(ratio > 0 && ratio <= 1 && Length(major) > 1e-12 && Math.Abs(Dot(Unit(major), normal)) < 1e-8, "неверный базис эллипса");
          Require(Math.Abs(Dot(center - planeOrigin, planeNormal)) <= 1e-6 && Math.Abs(Math.Abs(Dot(normal, planeNormal)) - 1) < 1e-8, "эллипс вне плоскости REGION");
          var minor = XYZ.Cross(normal, major) * ratio;
          Near(center + major * Math.Cos(a) + minor * Math.Sin(a), start);
          Near(center + major * Math.Cos(b) + minor * Math.Sin(b), end);
          var span = b - a;
          Require(span > 1e-12 && span <= 2 * Math.PI + 1e-8, "неподдержанный интервал эллипса");
          a %= 2 * Math.PI; if (a < 0) a += 2 * Math.PI;
          var worldMajor = Direction(major); var worldMinor = Direction(minor);
          // Полный эллипс в DXF имеет канонический интервал 0..2π независимо от начальной вершины SAB.
          var full = Math.Abs(span - 2 * Math.PI) <= 1e-12 || span > 2 * Math.PI;
          entity = new Ellipse { Center = Position(center), MajorAxisEndPoint = worldMajor, Normal = Unit(XYZ.Cross(worldMajor, worldMinor)), RadiusRatio = ratio, StartParameter = full ? 0 : a, EndParameter = full ? 2 * Math.PI : a + span };
        }
        else throw Error("неподдержанная кривая " + curve.Name);
        entity.Layer = region.Layer; entity.Color = region.Color; entity.IsInvisible = region.IsInvisible;
        entity.LineType = region.LineType; entity.LineTypeScale = region.LineTypeScale; entity.LineWeight = region.LineWeight;
        entity.Transparency = region.Transparency; entity.Material = region.Material;
        output.Add(entity);
        current = Ref(coedge, 3);
      } while (current != first);
      Require(firstPoint.HasValue && previousEnd.HasValue, "пустой контур"); Near(previousEnd!.Value, firstPoint!.Value);
      loopId = Ref(loop, 3);
    }
    Require(output.Count > 0, "нет контуров");
    Require(edges.Count == records.Count(r => r.Name == "edge") && coedges.Count == records.Count(r => r.Name == "coedge") && loops.Count == records.Count(r => r.Name == "loop"), "неполный обход топологии");
    for (var i = 0; i < records.Count; i++)
      Require(visited.Contains(i) || records[i].Name is "asmheader" or "ref_vt-eye-attrib" or "fmesh-eye-attrib", $"неохваченная запись {records[i].Name} #{i}");
    return output;
  }

  private static void Validate(Record record)
  {
    var schema = record.Name switch {
      "asmheader" => "ris", "body" or "lump" or "loop" => "rirrrr", "shell" => "rirrrrrr",
      "face" => "rirrrrrrbbb", "coedge" => "rirrrrrbrir", "edge" => "rirrdrdrrbs",
      "vertex" => "rirrir", "point" => "rirp", "transform" => "rivvvvdbbb",
      "ref_vt-eye-attrib" => "rirrrrr", "fmesh-eye-attrib" => "rirrr",
      "plane-surface" => "rirpvvbbbbb", "straight-curve" => "rirpv", "ellipse-curve" => "rirpvvd",
      _ => throw Error("неподдержанная запись " + record.Name)
    };
    var fields = record.Fields;
    Require(fields.Length >= schema.Length, "короткая запись " + record.Name);
    for (var i = 0; i < schema.Length; i++)
    {
      var tag = fields[i].Tag;
      Require(schema[i] switch { 'r' => tag == 12, 'i' => tag == 4, 's' => tag == 7, 'd' => tag == 6, 'v' => tag == 20, 'p' => tag == 19, 'b' => tag is 10 or 11, _ => false }, "неверная структура " + record.Name);
    }
    var index = schema.Length;
    if (record.Name is "straight-curve" or "ellipse-curve")
      for (var bound = 0; bound < 2; bound++)
      {
        Require(index < fields.Length && fields[index].Tag is 10 or 11, "неверная граница кривой");
        if ((bool)fields[index++].Value) { Require(index < fields.Length && fields[index].Tag == 6, "нет параметра границы"); index++; }
      }
    Require(index == fields.Length, "лишние поля " + record.Name);
    if (record.Name == "plane-surface") for (var i = 7; i < 11; i++) Require(!Flag(record, i), "ограниченная поверхность пока не поддерживается");
  }

  private static List<Record> Read(byte[] data)
  {
    try
    {
      using var stream = new MemoryStream(data, false); using var reader = new BinaryReader(stream, Encoding.ASCII);
      Require(Encoding.ASCII.GetString(reader.ReadBytes(15)) == "ASM BinaryFile4", "ожидался бинарный ASM");
      Require(reader.ReadInt32() == 22300, "поддерживается версия SAB 22300");
      reader.ReadInt32(); reader.ReadInt32(); Require(reader.ReadInt32() == 4, "неподдержанные флаги заголовка");
      string String()
      {
        var size = reader.ReadByte(); var bytes = reader.ReadBytes(size); Require(bytes.Length == size, "оборванная строка");
        return Encoding.ASCII.GetString(bytes);
      }
      double Number() { var value = reader.ReadDouble(); Require(double.IsFinite(value), "нечисловая координата"); return value; }
      for (var i = 0; i < 3; i++) { Require(reader.ReadByte() == 7, "строка заголовка"); String(); }
      for (var i = 0; i < 3; i++) { Require(reader.ReadByte() == 6, "число заголовка"); var value = Number(); if (i == 0) Require(value == 1, "единицы SAB отличаются от миллиметров"); }
      var result = new List<Record>();
      while (stream.Position < stream.Length)
      {
        var parts = new List<string>(); byte tag;
        do { tag = reader.ReadByte(); Require(tag is 13 or 14, "имя записи"); parts.Add(String()); } while (tag == 14);
        var name = string.Join("-", parts);
        if (name == "End-of-ASM-data") { Require(stream.Position == stream.Length, "лишние данные после маркера завершения"); return result; }
        var fields = new List<Token>();
        while ((tag = reader.ReadByte()) != 17)
        {
          object value = tag switch {
            4 or 12 => reader.ReadInt32(), 6 => Number(), 7 => String(), 10 => true, 11 => false,
            19 or 20 => new XYZ(Number(), Number(), Number()),
            _ => throw Error($"неподдержанный тег {tag} в {name}")
          };
          fields.Add(new(tag, value));
        }
        result.Add(new(name, fields.ToArray()));
      }
      throw Error("нет маркера завершения");
    }
    catch (EndOfStreamException) { throw Error("оборванные данные"); }
  }
}
