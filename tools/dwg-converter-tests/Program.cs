using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Entities.AecEntities;
using ACadSharp.Entities.ProxyGraphics;
using ACadSharp.Tables;
using CSMath;
using System.Reflection;
using ACadSharp.IO;
using System.Text;

Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);

var tests = new (string Name, Action Run)[] {
  ("единственная замена и сохранение обычных сущностей", () => {
    var doc = new CadDocument();
    var wall = Add(doc, new ProxyPolyline { Points = [new(1,2,3), new(4,5,6)] });
    var ordinary = new Line(new(9,9,9),new(10,10,10));
    ordinary.ProxyGeometries.Add(new ProxyCircle { Center=new(100,100,0), Radius=50, Normal=XYZ.AxisZ });
    doc.Entities.Add(ordinary);
    Convert(doc);
    Check(doc.Entities.Count == 2 && doc.Entities.Contains(ordinary) && !doc.Entities.Contains(wall), "дублирование или потеря исходного объекта");
    var line = (Line)doc.Entities.OfType<Insert>().Single().Block.Entities.Single();
    Check(line.StartPoint == new XYZ(1,2,3) && line.EndPoint == new XYZ(4,5,6), "координаты линии");
    Convert(doc);
    Check(doc.Entities.Count == 2, "повторное преобразование изменило результат");
  }),
  ("вложенные матрицы, восстановление и свойства", () => {
    var doc = new CadDocument();
    var layer = new Layer("Графика"); doc.Layers.Add(layer);
    var translate = Matrix4.Identity; translate.M03=10; translate.M13=20; translate.M23=30;
    var scale = Matrix4.Identity; scale.M00=2; scale.M11=2; scale.M22=2;
    var wall = Add(doc, new ProxyPushModelTransform { TransformationMatrix=translate }, new ProxyPushModelTransform { TransformationMatrix=scale },
      new ProxySubentLayer { LayerIndex=1 }, new ProxySubentTrueColor { Color=new Color(12,34,56), ColorMethod=ProxyColorMethod.ByColor },
      new ProxySubentLineWeight { LineWeight=LineWeightType.W25 },
      new ProxyPolylineWithNormal { Normal=XYZ.AxisZ, Points=[new(1,2,3),new(2,2,3)] }, new ProxyPopModelTransform(), new ProxyPopModelTransform(),
      new ProxyPolyline { Points=[new(0,0,0),new(1,0,0)] });
    wall.IsInvisible=true;
    Convert(doc);
    var insert=(Insert)doc.Entities.Single(); var lines=insert.Block.Entities.Cast<Line>().ToArray();
    Check(lines[0].StartPoint == new XYZ(12,24,36) && lines[0].EndPoint == new XYZ(14,24,36), "матрица должна применяться один раз");
    Check(lines[1].EndPoint == new XYZ(1,0,0), "стек матриц не восстановлен");
    Check(insert.IsInvisible && lines[0].Layer.Name=="Графика" && lines[0].Color.Equals(new Color(12,34,56)) && (int)lines[0].LineWeight==25, "потеря свойств");
  }),
  ("дуга, окружность и кириллица", () => {
    var doc=new CadDocument();
    Add(doc,new ProxyCircle { Center=new(3,4,5), Normal=XYZ.AxisZ, Radius=7 },
      new ProxyCircularArc { Center=new(0,0,2), Normal=XYZ.AxisZ, Radius=4, StartVectorDirection=XYZ.AxisY, SweepAngle=Math.PI/2 },
      new ProxyUnicodeText2 { Text="Стена", StartPoint=new(7,8,9), Normal=XYZ.AxisZ, TextDirection=XYZ.AxisY, Height=2, WidthFactor=0.8, TrackingPercentage=1, FontDescriptor=new() { Typeface="Arial", FontFilename="arial.ttf" } });
    Convert(doc); var ents=((Insert)doc.Entities.Single()).Block.Entities;
    var c=ents.OfType<Circle>().First(e=>e is not Arc); Check(c.Center==new XYZ(3,4,5)&&c.Radius==7,"окружность");
    var a=ents.OfType<Arc>().Single(); Check(Math.Abs(a.StartAngle-Math.PI/2)<1e-9 && Math.Abs(a.EndAngle-Math.PI)<1e-9,"углы дуги");
    var t=ents.OfType<TextEntity>().Single(); Check(t.Value=="Стена" && t.InsertPoint==new XYZ(7,8,9) && Math.Abs(t.Rotation-Math.PI/2)<1e-9 && t.Style.Filename=="arial.ttf", "текст и шрифт");
  }),
  ("невыпуклая заливка и оболочка", () => {
    var doc=new CadDocument();
    Add(doc,new ProxySubentFillon { IsOn=true },new ProxyPolygon { Points=[new(0,0,0),new(3,0,0),new(3,1,0),new(1,1,0),new(1,3,0),new(0,3,0)] },
      new ProxyShell { Vertices=[new(10,0,0),new(11,0,0),new(11,1,0),new(10,1,0)], Faces=[new[]{0,1,2,3}] });
    Convert(doc); var ents=((Insert)doc.Entities.Single()).Block.Entities;
    var area=ents.OfType<Solid>().Sum(s=>Math.Abs((s.SecondCorner.X-s.FirstCorner.X)*(s.ThirdCorner.Y-s.FirstCorner.Y)-(s.SecondCorner.Y-s.FirstCorner.Y)*(s.ThirdCorner.X-s.FirstCorner.X))/2);
    Check(Math.Abs(area-6)<1e-9,"неверная площадь невыпуклой заливки");
    Check(!ents.OfType<Face3D>().Any() && ents.OfType<Line>().Count()==4,"оболочка содержит видимые диагонали или потеряла границы");
  }),
  ("неподдержанная команда не меняет документ", () => {
    var doc=new CadDocument(); var good=Add(doc,new ProxyPolyline { Points=[new(0,0,0),new(1,0,0)] });
    var bad=Add(doc,new ProxyPushClip()); var count=doc.BlockRecords.Count;
    try { Convert(doc); throw new Exception("неподдержанная команда принята"); }
    catch(NotSupportedException) { }
    Check(doc.Entities.Contains(good)&&doc.Entities.Contains(bad)&&doc.BlockRecords.Count==count,"частичное изменение документа после ошибки");
  }),
  ("одноточечная полилиния сохраняется как точка", () => {
    var doc=new CadDocument(); Add(doc,new ProxyPolyline { Points=[new(4,5,6)] }); Convert(doc);
    Check(((Insert)doc.Entities.Single()).Block.Entities.Single() is ACadSharp.Entities.Point point && point.Location==new XYZ(4,5,6),"точка потеряна");
  }),
  ("форматирование текста и межбуквенный интервал", () => {
    var doc=new CadDocument(); Add(doc,new ProxyUnicodeText2 { Text="Размер", StartPoint=new(1,2,3), Normal=XYZ.AxisZ, TextDirection=XYZ.AxisX, Height=4, WidthFactor=0.8, TrackingPercentage=0.95, IsUnderlined=true, FontDescriptor=new() { Typeface="Arial", FontFilename="arial.ttf" } }); Convert(doc);
    var text=((Insert)doc.Entities.Single()).Block.Entities.OfType<MText>().Single();
    Check(text.Value.Contains(@"\T0.94999999999999996;") && text.Value.Contains(@"\L") && text.InsertPoint==new XYZ(1,2,3) && text.Style.Name.StartsWith("DWG_PROXY_BASELINE_"),"форматирование или привязка к базовой линии потеряны");
  }),
  ("таблица и повторные вставки не удваивают преобразование", () => {
    var doc=new CadDocument(); var table=new TableEntity(new BlockRecord("ТелоТаблицы")) { InsertPoint=new(1000,2000,0), XScale=10,YScale=10 };
    table.ProxyGeometries.Add(new ProxyPolyline { Points=[new(10,20,0),new(15,20,0)] });
    var block=new BlockRecord("Повторяемый"); block.Entities.Add(table); doc.BlockRecords.Add(block);
    doc.Entities.Add(new Insert(block) { InsertPoint=new(1,0,0) }); doc.Entities.Add(new Insert(block) { InsertPoint=new(2,0,0) });
    Convert(doc); var replacement=block.Entities.OfType<Insert>().Single();
    Check(replacement.InsertPoint==XYZ.Zero && replacement.XScale==1 && replacement.Block.Entities.OfType<Line>().Single().StartPoint==new XYZ(10,20,0),"трансформация таблицы применена повторно");
    Check(doc.Entities.Count==2,"потеря повторной вставки");
  }),
  ("эллиптическая дуга сохраняет параметры и ориентацию", () => {
    var doc=new CadDocument(); Add(doc,new ProxyEllipseGraphic { Center=new(10,20,30), Normal=XYZ.AxisZ, MajorRadius=5,MinorRadius=2,MajorAngle=Math.PI/2,StartParameter=0.25,EndParameter=1.5 }); Convert(doc);
    var ellipse=((Insert)doc.Entities.Single()).Block.Entities.OfType<Ellipse>().Single();
    Check(ellipse.Center==new XYZ(10,20,30) && Math.Abs(ellipse.MajorAxisEndPoint.Y-5)<1e-9 && Math.Abs(ellipse.RadiusRatio-0.4)<1e-9 && ellipse.StartParameter==0.25 && ellipse.EndParameter==1.5,"эллиптическая дуга потеряла параметры");
  }),
  ("пустая сохранённая графика остаётся учтённой вставкой", () => {
    var doc=new CadDocument(); var source=Add(doc,new ProxySubentMarker { MarkerIndex=1 }); var handle=source.Handle; var report=ProxyConversion.Convert(doc);
    Check(report.SourceCount==2 && report.ReplacedCount==1 && report.GeneratedCount==0 && report.Mappings.Single().SourceHandle==handle,$"неверный учёт пустой графики: {report.SourceCount}/{report.ReplacedCount}/{report.GeneratedCount}, handle {report.Mappings.Single().SourceHandle}/{handle}");
    Check(doc.Entities.Single() is Insert insert && insert.Block.Entities.Count==0,"пустой объект пропал из учёта");
  }),
  ("атрибут 51 сохраняет известные режимы прозрачности", () => {
    var doc=new CadDocument(); Add(doc,new ProxyRawAttribute51 { RawValue=0 },new ProxyPolyline { Points=[new(0,0,0),new(1,0,0)] },new ProxyRawAttribute51 { RawValue=0x020000FF },new ProxyPolyline { Points=[new(2,0,0),new(3,0,0)] }); Convert(doc);
    var lines=((Insert)doc.Entities.Single()).Block.Entities.OfType<Line>().ToArray();
    Check(lines[0].Transparency.IsByLayer && Transparency.ToAlphaValue(lines[1].Transparency)==0x020000FF,"потерян режим прозрачности");
    var bad=new CadDocument(); var source=Add(bad,new ProxyRawAttribute51 { RawValue=123 });
    try { Convert(bad); throw new Exception("неизвестное значение принято"); } catch(NotSupportedException) { }
    Check(bad.Entities.Contains(source),"источник удалён при неизвестном значении");
  }),
  ("видимость рёбер оболочки использует 0 скрыто и 1 видно", () => {
    var doc=new CadDocument(); var edges=new EdgeTraits(); edges.VisibilityIndicators.AddRange([0,1,1,0]);
    Add(doc,new ProxyShell { Vertices=[new(0,0,0),new(2,0,0),new(2,2,0),new(0,2,0)], Faces=[new[]{0,1,2,3}], EdgeTraits=edges }); Convert(doc);
    var lines=((Insert)doc.Entities.Single()).Block.Entities.OfType<Line>().ToArray();
    Check(lines.Length==2 && lines[0].StartPoint==new XYZ(2,0,0) && lines[0].EndPoint==new XYZ(2,2,0) && lines[1].StartPoint==new XYZ(2,2,0),"инвертированная видимость рёбер");
  }),
  ("мультилиния имеет единственное представление и учёт", () => {
    var doc=new CadDocument(); var style=new ACadSharp.Objects.MLineStyle("Проверка") { Flags=ACadSharp.Objects.MLineStyleFlags.None };
    style.AddElement(new ACadSharp.Objects.MLineStyle.Element { Offset=1, Color=new Color(1), LineType=LineType.Continuous });
    style.AddElement(new ACadSharp.Objects.MLineStyle.Element { Offset=-1, Color=new Color(1), LineType=LineType.Continuous });
    var source=new MLine { Style=style, Flags=MLineFlags.HasVertices, ScaleFactor=10 };
    foreach(var position in new[]{new XYZ(0,0,0),new XYZ(10,0,0)}) {
      var vertex=new MLine.Vertex { Position=position, Direction=XYZ.AxisX, Miter=XYZ.AxisY };
      foreach(var distance in new[]{1.0,-1.0}) { var segment=new MLine.Vertex.Segment(); segment.Parameters.AddRange([distance,0]); vertex.Segments.Add(segment); }
      source.Vertices.Add(vertex);
    }
    doc.Entities.Add(source); var handle=source.Handle; var report=ProxyConversion.Convert(doc);
    Check(!doc.Entities.Contains(source)&&doc.Entities.Single() is Insert insert&&insert.Block.Entities.OfType<Line>().Count()==2,"мультилиния не заменена единственным блоком");
    Check(report.Mappings.Single().SourceHandle==handle&&report.GeneratedCount==2,"нет учёта мультилинии");
  }),
  ("запись DXF сохраняет наклонные плоскости и координаты", () => {
    var doc=new CadDocument();
    Add(doc,new ProxyCircle { Center=new(1,2,3), Normal=XYZ.AxisY, Radius=4 },new ProxyUnicodeText2 { Text="Наклон", StartPoint=new(5,6,7),Normal=XYZ.AxisY,TextDirection=-XYZ.AxisX,Height=2,WidthFactor=1,TrackingPercentage=1,FontDescriptor=new(){FontFilename="arial.ttf",Typeface="Arial"} });
    var report=ProxyConversion.Convert(doc); using var stream=new MemoryStream(); DxfWriter.Write(stream,doc,false,new DxfWriterConfiguration());
    using var input=new MemoryStream(stream.ToArray()); var written=DxfReader.Read(input);
    var insert=written.Entities.OfType<Insert>().Single(); var circle=insert.Block.Entities.OfType<Circle>().Single(); var text=insert.Block.Entities.OfType<TextEntity>().Single();
    Check(circle.Center==new XYZ(-1,3,2)&&circle.Normal==XYZ.AxisY&&circle.Radius==4,"неверные координаты окружности в плоскости DXF");
    Check(text.InsertPoint==new XYZ(-5,7,6)&&text.Normal==XYZ.AxisY&&text.Value=="Наклон","неверная плоскость текста DXF");
    Check(insert.Block.Entities.Count==2&&report.GeneratedCount==2,"запись продублировала геометрию");
  }),
  ("нет графики — явная ошибка", () => {
    var doc=new CadDocument(); var wall=Add(doc);
    try { Convert(doc); throw new Exception("пустая графика принята"); } catch(NotSupportedException) { }
    Check(doc.Entities.Contains(wall),"исходник удалён");
  })
};
var failed=0;
foreach(var test in tests) { try { test.Run(); Console.WriteLine("PASS: "+test.Name); } catch(Exception ex) { failed++; Console.Error.WriteLine("FAIL: "+test.Name+": "+ex.Message); } }
return failed==0?0:1;
static Wall Add(CadDocument doc, params IProxyGeometry[] commands) { var wall=new Wall(); wall.ProxyGeometries.AddRange(commands); doc.Entities.Add(wall); return wall; }
static void Check(bool condition,string message) { if(!condition) throw new Exception(message); }
static void Convert(CadDocument doc) {
  var type=Assembly.GetExecutingAssembly().GetType("ProxyConversion");
  if(type==null) throw new Exception("отсутствует преобразование сохранённой графики");
  try { type.GetMethod("Convert")!.Invoke(null,[doc]); } catch(TargetInvocationException ex) when(ex.InnerException!=null) { System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(ex.InnerException).Throw(); }
}
