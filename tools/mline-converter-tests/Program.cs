using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Objects;
using ACadSharp.Tables;
using CSMath;

void Assert(bool result, string text) { if (!result) throw new Exception(text); }
void Near(double a, double b, string text) => Assert(Math.Abs(a - b) < 1e-8, text);
MLine Bent()
{
  var style = new MLineStyle("Тест") { Flags = MLineStyleFlags.FillOn | MLineStyleFlags.StartSquareCap | MLineStyleFlags.EndSquareCap, FillColor = new Color(3) };
  style.AddElement(new MLineStyle.Element { Offset = 1, Color = new Color(1), LineType = LineType.Continuous });
  style.AddElement(new MLineStyle.Element { Offset = -1, Color = new Color(1), LineType = LineType.Continuous });
  var source = new MLine { Style = style, Layer = new Layer("Проверка"), ScaleFactor = 27, Flags = MLineFlags.HasVertices };
  foreach (var (position, miter) in new[] { (new XYZ(0, 0, 5), XYZ.AxisY), (new XYZ(10, 0, 5), new XYZ(-1, 1, 0)), (new XYZ(10, 10, 5), -XYZ.AxisX) })
  {
    var vertex = new MLine.Vertex { Position = position, Miter = miter, Direction = XYZ.AxisX };
    foreach (var distance in new[] { 1.0, -1.0 })
    {
      var segment = new MLine.Vertex.Segment();
      segment.Parameters.AddRange(new[] { distance, 0 });
      vertex.Segments.Add(segment);
    }
    source.Vertices.Add(vertex);
  }
  return source;
}

var source = Bent();
var generated = MLineConversion.Build(source);
var solids = generated.OfType<Solid>().ToArray();
var lines = generated.OfType<Line>().ToArray();
Assert(solids.Length == 4 && lines.Length == 6, "Г-образная линия: неверное число заливок или границ.");
double Area(Solid face)
{
  var a = face.SecondCorner - face.FirstCorner;
  var b = face.ThirdCorner - face.FirstCorner;
  var c = XYZ.Cross(a, b);
  return Math.Sqrt(c.X * c.X + c.Y * c.Y + c.Z * c.Z) / 2;
}
Near(solids.Sum(Area), 40, "Г-образная линия: площадь должна быть 40 без наложения заливки.");
Assert(lines.Any(l => l.StartPoint == new XYZ(0, 1, 5) && l.EndPoint == new XYZ(9, 1, 5)), "Параметры вершины повторно масштабированы или потерян Z.");
Assert(generated.All(e => ReferenceEquals(e.Layer, source.Layer)), "Изменился слой.");
Assert(solids.All(e => e.Color.Equals(source.Style.FillColor)) && lines.All(e => e.Color.Equals(source.Style.Elements.First().Color)), "Потерян цвет заливки или границы.");
Assert(source.Vertices.Count == 3 && source.ScaleFactor == 27, "Изменён исходник.");
Console.WriteLine("PASS: площадь, окончательные координаты, Z, стили и отсутствие повторного масштаба.");

var tilted = Bent();
XYZ Tilt(XYZ p) => new(p.X, p.Z, -p.Y);
tilted.Normal = XYZ.AxisY;
tilted.ScaleFactor = -99;
tilted.LineTypeScale = 3;
tilted.LineWeight = LineWeightType.W50;
foreach (var vertex in tilted.Vertices)
{
  vertex.Position = Tilt(vertex.Position);
  vertex.Miter = Tilt(vertex.Miter);
  vertex.Direction = Tilt(vertex.Direction);
}
var tiltedOutput = MLineConversion.Build(tilted);
Assert(tiltedOutput.OfType<Line>().Any(l => l.StartPoint == new XYZ(0, 5, -1) && l.EndPoint == new XYZ(9, 5, -1)), "Наклонная плоскость: потеряны мировые координаты.");
Assert(tiltedOutput.OfType<Solid>().All(s => s.Normal == XYZ.AxisY && Math.Abs(s.FirstCorner.Z - 5) < 1e-8), "Наклонная заливка: потеряны нормаль или высота плоскости.");
Assert(tiltedOutput.All(e => e.LineTypeScale == 3 && e.LineWeight == LineWeightType.W50), "Потеряны толщина или масштаб типа линии.");
Near(tiltedOutput.OfType<Solid>().Sum(Area), 40, "Наклонная заливка: площадь изменилась.");
Console.WriteLine("PASS: произвольная плоскость, отрицательный масштаб и параметры линии.");

source.Flags |= MLineFlags.NoStartCaps | MLineFlags.NoEndCaps;
Assert(MLineConversion.Build(source).OfType<Line>().Count() == 4, "Подавленные торцы остались.");
source.Flags = MLineFlags.HasVertices | MLineFlags.Closed;
var closed = MLineConversion.Build(source);
Assert(closed.OfType<Line>().Count() == 6 && closed.OfType<Solid>().Count() == 6, "Закрытый контур: не замкнуты границы или добавлены торцы.");
Console.WriteLine("PASS: подавление торцов и замыкание границ.");

source = Bent();
source.Style.Elements.Last().Color = new Color(2);
Assert(MLineConversion.Build(source).OfType<Line>().Count() == 8, "Разноцветный торец не разделён на две половины.");
source.Style.Flags |= MLineStyleFlags.DisplayJoints;
Assert(MLineConversion.Build(source).OfType<Line>().Count() == 10, "Стык должен добавляться один раз.");
Console.WriteLine("PASS: цвета половин торцов и единственный внутренний стык.");

void MustFail(MLine input)
{
  var failed = false;
  try { MLineConversion.Build(input); } catch (NotSupportedException) { failed = true; }
  Assert(failed, "Неподдерживаемый случай ошибочно принят.");
}
source = Bent();
source.Vertices[0].Segments[0].Parameters.Add(2);
MustFail(source);
source = Bent();
source.Style.Flags |= MLineStyleFlags.StartRoundCap;
MustFail(source);
source = Bent();
source.Vertices[0].Segments[0].AreaFillParameters.Add(1);
MustFail(source);
Console.WriteLine("PASS: разрывы, круглый торец и разрыв заливки явно отклонены.");
