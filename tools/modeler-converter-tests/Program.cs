using ACadSharp.Entities;
using CSMath;
using System.Text;
using System.Text.Json;

void Assert(bool condition, string message) { if (!condition) throw new Exception(message); }
double Length(XYZ v) => Math.Sqrt(v.X * v.X + v.Y * v.Y + v.Z * v.Z);
void Near(XYZ actual, XYZ expected) => Assert(Length(actual - expected) < 1e-7, $"Координаты {actual} != {expected}");
void Reject(byte[] data, string description)
{
  try { ModelerConversion.Build(new Region { AcisData = data }); }
  catch (NotSupportedException) { return; }
  throw new Exception("Не отклонено: " + description);
}

var fixture = new Fixture();
var region = new Region { AcisData = fixture.Bytes(), Color = new ACadSharp.Color(3) };
var result = ModelerConversion.Build(region);
Assert(result.Count == 5 && result.OfType<Line>().Count() == 2 && result.OfType<Ellipse>().Count() == 3, "Потеря внешнего контура или отверстия.");
var line = result.OfType<Line>().First();
Near(line.StartPoint, new XYZ(12, 20, 3)); Near(line.EndPoint, new XYZ(12, 28, 3));
var arc = result.OfType<Ellipse>().First();
Near(arc.Center, new XYZ(10, 28, 3));
Assert(Math.Abs(Length(arc.MajorAxisEndPoint) - 2) < 1e-8 && Math.Abs(arc.EndParameter - arc.StartParameter - Math.PI) < 1e-8, "Потеря радиуса или полуокружности.");
Assert(result.All(e => e.Color.Equals(region.Color)), "Не сохранён стиль.");
Assert(region.AcisData.SequenceEqual(fixture.Bytes()), "Изменён исходный SAB.");
Console.WriteLine("PASS: аналитические линии/дуги, отверстие, преобразование, обратная coedge, стиль.");

Reject(fixture.Bytes()[..^1], "оборванный SAB");
Reject(Encoding.ASCII.GetBytes("700 0 1 0"), "неподдерживаемый SAT");
var unknown = new Fixture(); unknown.Records[11].Name = "spline-curve"; Reject(unknown.Bytes(), "неизвестная кривая");
var disconnected = new Fixture(); disconnected.Records[6].Fields[3] = Fixture.Ref(6); Reject(disconnected.Bytes(), "разорванный цикл coedge");
var wrongPoint = new Fixture(); wrongPoint.Records[16].Fields[3] = Fixture.Vec(19, 100, 100, 0); Reject(wrongPoint.Bytes(), "вершина вне кривой");
var orphan = new Fixture(); orphan.Records.Add(new("point", Fixture.Common().Concat(new[] { Fixture.Vec(19, 9, 9, 0) }).ToArray())); Reject(orphan.Bytes(), "неохваченная геометрия");
Console.WriteLine("PASS: повреждения и непокрытая геометрия явно отклонены.");

var reflected = new Fixture();
reflected.Records[24].Fields[2] = Fixture.Vec(20, 0, -2, 0);
reflected.Records[24].Fields[8] = new(10, true);
var reflectedArc = ModelerConversion.Build(new Region { AcisData = reflected.Bytes() }).OfType<Ellipse>().First();
Near(reflectedArc.Normal, -XYZ.AxisZ);
Near(reflectedArc.Center, new XYZ(10, 12, 3));
var tilted = new Fixture();
tilted.Records[24].Fields[3] = Fixture.Vec(20, 0, 0, -2);
tilted.Records[24].Fields[4] = Fixture.Vec(20, -2, 0, 0);
Near(ModelerConversion.Build(new Region { AcisData = tilted.Bytes() }).OfType<Ellipse>().First().Normal, -XYZ.AxisX);
var oval = new Fixture(); oval.Records[29].Fields[6] = new(6, 0.5);
Assert(ModelerConversion.Build(new Region { AcisData = oval.Bytes() }).OfType<Ellipse>().Last().RadiusRatio == 0.5, "Эллипс заменён окружностью.");
var offPlane = new Fixture(); offPlane.Records[29].Fields[4] = Fixture.Vec(20, 0, 1, 0); Reject(offPlane.Bytes(), "кривая выходит из плоскости между вершинами");
var nonfinite = new Fixture(); nonfinite.Records[16].Fields[3] = Fixture.Vec(19, double.NaN, 0, 0); Reject(nonfinite.Bytes(), "нечисловая координата");
Console.WriteLine("PASS: отражение, наклонная плоскость, эллипс и проверка всей кривой.");

var doc = new ACadSharp.CadDocument();
doc.Entities.Add(new Region { AcisData = new Fixture().Bytes() });
var converted = ProxyConversion.Convert(doc);
Assert(converted.ReplacedCount == 1 && converted.GeneratedCount == 5 && doc.Entities.Count == 1 && doc.Entities.First() is Insert, "REGION не заменён единственным INSERT.");
var badDoc = new ACadSharp.CadDocument();
badDoc.Entities.Add(new Region { AcisData = new Fixture().Bytes() });
badDoc.Entities.Add(new Region { AcisData = new byte[] { 1, 2 } });
try { ProxyConversion.Convert(badDoc); throw new Exception("Повреждённый REGION пропущен."); }
catch (NotSupportedException) { Assert(badDoc.Entities.Count == 2 && badDoc.Entities.All(e => e is Region), "Частичная конверсия удалила исходник."); }
Console.WriteLine("PASS: одна замена и отсутствие частичных изменений при ошибке.");

var reviewFailures = new List<string>();
var shiftedFull = new Fixture();
shiftedFull.Records[28].Fields[4] = new(6, 1.0);
shiftedFull.Records[28].Fields[6] = new(6, 1.0 + 2 * Math.PI);
shiftedFull.Records[33].Fields[3] = Fixture.Vec(19, 2 + 0.25 * Math.Cos(1), 0.25 * Math.Sin(1), 0);
var fullEllipse = ModelerConversion.Build(new Region { AcisData = shiftedFull.Bytes() }).OfType<Ellipse>().Last();
if (fullEllipse.StartParameter != 0 || fullEllipse.EndParameter != 2 * Math.PI || !fullEllipse.IsFullEllipse)
  reviewFailures.Add("Полный эллипс со смещённым интервалом не приведён к 0/2π.");
var excessiveScale = new Fixture();
excessiveScale.Records[24].Fields[2] = Fixture.Vec(20, 0, 1e154, 0);
excessiveScale.Records[24].Fields[3] = Fixture.Vec(20, -1e154, 0, 0);
excessiveScale.Records[24].Fields[4] = Fixture.Vec(20, 0, 0, 1e154);
excessiveScale.Records[24].Fields[6] = new(6, 1e154);
try { Reject(excessiveScale.Bytes(), "переполнение промежуточной нормы"); }
catch (Exception ex) { reviewFailures.Add(ex.Message); }
Assert(reviewFailures.Count == 0, string.Join(Environment.NewLine, reviewFailures));
Console.WriteLine("PASS: канонический полный эллипс и отказ при переполнении.");

if (args.Length != 0)
{
  var count = 0; var edges = 0; var lines = 0; var ellipses = 0;
  using var input = JsonDocument.Parse(File.ReadAllText(args[0]));
  foreach (var item in input.RootElement.EnumerateArray())
  {
    var data = Convert.FromBase64String(item.GetProperty("data").GetString()!);
    var entities = ModelerConversion.Build(new Region { AcisData = data });
    count++; edges += entities.Count; lines += entities.OfType<Line>().Count(); ellipses += entities.OfType<Ellipse>().Count();
  }
  Assert(count == 812 && edges == 38052 && lines == 18368 && ellipses == 19684, "Неполное покрытие 812 REGION.");
  Console.WriteLine($"PASS: REGION={count}; рёбер={edges}; линий={lines}; эллиптических дуг={ellipses}.");
}

sealed class Fixture
{
  public sealed record Field(byte Tag, object Value);
  public sealed class Record(string name, params Field[] fields) { public string Name = name; public Field[] Fields = fields; }
  public List<Record> Records = new();
  public static Field Ref(int value) => new(12, value);
  static Field Int(int value) => new(4, value);
  static Field Real(double value) => new(6, value);
  static Field Flag(bool value) => new((byte)(value ? 10 : 11), value);
  static Field Text(string value) => new(7, value);
  public static Field Vec(byte tag, double x, double y, double z) => new(tag, new[] { x, y, z });
  public static Field[] Common() => new[] { Ref(-1), Int(-1), Ref(-1) };
  void Add(string name, params Field[] data) => Records.Add(new(name, Common().Concat(data).ToArray()));
  public Fixture()
  {
    Records.Add(new("asmheader", Ref(-1), Int(-1), Text("228.5.0.65535")));
    Add("body", Ref(2), Ref(-1), Ref(24));
    Add("lump", Ref(-1), Ref(3), Ref(1));
    Add("shell", Ref(-1), Ref(-1), Ref(4), Ref(-1), Ref(2));
    Add("face", Ref(-1), Ref(5), Ref(3), Ref(-1), Ref(25), Flag(false), Flag(true), Flag(false));
    Add("loop", Ref(26), Ref(6), Ref(4));
    Add("coedge", Ref(7), Ref(9), Ref(-1), Ref(10), Flag(false), Ref(5), Int(0), Ref(-1));
    Add("coedge", Ref(8), Ref(6), Ref(-1), Ref(12), Flag(false), Ref(5), Int(0), Ref(-1));
    Add("coedge", Ref(9), Ref(7), Ref(-1), Ref(18), Flag(true), Ref(5), Int(0), Ref(-1));
    Add("coedge", Ref(6), Ref(8), Ref(-1), Ref(20), Flag(false), Ref(5), Int(0), Ref(-1));
    Add("edge", Ref(14), Real(0), Ref(15), Real(4), Ref(6), Ref(11), Flag(false), Text("unknown"));
    Add("straight-curve", Vec(19, 0, -1, 0), Vec(20, 1, 0, 0), Flag(false), Flag(false));
    Add("edge", Ref(15), Real(-Math.PI / 2), Ref(22), Real(Math.PI / 2), Ref(7), Ref(13), Flag(false), Text("unknown"));
    Add("ellipse-curve", Vec(19, 4, 0, 0), Vec(20, 0, 0, 1), Vec(20, 1, 0, 0), Real(1), Flag(false), Flag(false));
    Add("vertex", Ref(10), Int(2), Ref(16));
    Add("vertex", Ref(10), Int(2), Ref(17));
    Add("point", Vec(19, 0, -1, 0)); Add("point", Vec(19, 4, -1, 0));
    Add("edge", Ref(23), Real(0), Ref(22), Real(4), Ref(8), Ref(19), Flag(false), Text("unknown"));
    Add("straight-curve", Vec(19, 0, 1, 0), Vec(20, 1, 0, 0), Flag(false), Flag(false));
    Add("edge", Ref(23), Real(Math.PI / 2), Ref(14), Real(3 * Math.PI / 2), Ref(9), Ref(21), Flag(false), Text("unknown"));
    Add("ellipse-curve", Vec(19, 0, 0, 0), Vec(20, 0, 0, 1), Vec(20, 1, 0, 0), Real(1), Flag(false), Flag(false));
    Add("vertex", Ref(12), Int(2), Ref(30)); Add("vertex", Ref(18), Int(2), Ref(31));
    Records.Add(new("transform", Ref(-1), Int(-1), Vec(20, 0, 2, 0), Vec(20, -2, 0, 0), Vec(20, 0, 0, 2), Vec(20, 10, 20, 3), Real(2), Flag(true), Flag(false), Flag(false)));
    Add("plane-surface", Vec(19, 0, 0, 0), Vec(20, 0, 0, 1), Vec(20, 1, 0, 0), Flag(false), Flag(false), Flag(false), Flag(false), Flag(false));
    Add("loop", Ref(-1), Ref(27), Ref(4));
    Add("coedge", Ref(27), Ref(27), Ref(-1), Ref(28), Flag(true), Ref(26), Int(0), Ref(-1));
    Add("edge", Ref(32), Real(0), Ref(32), Real(2 * Math.PI), Ref(27), Ref(29), Flag(false), Text("unknown"));
    Add("ellipse-curve", Vec(19, 2, 0, 0), Vec(20, 0, 0, 1), Vec(20, 0.25, 0, 0), Real(1), Flag(false), Flag(false));
    Add("point", Vec(19, 4, 1, 0)); Add("point", Vec(19, 0, 1, 0));
    Add("vertex", Ref(28), Int(2), Ref(33)); Add("point", Vec(19, 2.25, 0, 0));
  }
  public byte[] Bytes()
  {
    using var stream = new MemoryStream(); using var writer = new BinaryWriter(stream);
    writer.Write(Encoding.ASCII.GetBytes("ASM BinaryFile4"));
    writer.Write(22300); writer.Write(0); writer.Write(2); writer.Write(4);
    void Write(Field field)
    {
      writer.Write(field.Tag);
      if (field.Tag is 4 or 12) writer.Write((int)field.Value);
      else if (field.Tag == 6) writer.Write((double)field.Value);
      else if (field.Tag == 7) { var bytes = Encoding.ASCII.GetBytes((string)field.Value); writer.Write((byte)bytes.Length); writer.Write(bytes); }
      else if (field.Tag is 19 or 20) foreach (var value in (double[])field.Value) writer.Write(value);
    }
    Write(Text("Synthetic fixture")); Write(Text("ASM 228.5")); Write(Text("Tue Nov 03 00:00:00 2026"));
    Write(Real(1)); Write(Real(1e-6)); Write(Real(1e-10));
    void Name(string name)
    {
      var parts = name.Split('-');
      for (var i = 0; i < parts.Length; i++) { writer.Write((byte)(i == parts.Length - 1 ? 13 : 14)); writer.Write((byte)parts[i].Length); writer.Write(Encoding.ASCII.GetBytes(parts[i])); }
    }
    foreach (var record in Records) { Name(record.Name); foreach (var field in record.Fields) Write(field); writer.Write((byte)17); }
    Name("End-of-ASM-data");
    return stream.ToArray();
  }
}
