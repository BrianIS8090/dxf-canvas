using ACadSharp;
using ACadSharp.IO;
using ACadSharp.Entities.ProxyGraphics;
using CSMath;
using System.Text;
using System.Security.Cryptography;
Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
int passed = 0;
void Check(bool condition, string message) { if (!condition) throw new Exception(message); passed++; }
void Reject(Action action, string message) { try { action(); } catch (IOException) { passed++; return; } throw new Exception(message); }
byte[] Stream(params (int type, byte[] data)[] commands) {
  using var s = new MemoryStream(); using var w = new BinaryWriter(s);
  w.Write(8 + commands.Sum(c=>8+c.data.Length)); w.Write(commands.Length);
  foreach(var c in commands) { w.Write(8+c.data.Length); w.Write(c.type); w.Write(c.data); }
  return s.ToArray();
}
byte[] Ellipse() {
  using var s = new MemoryStream(); using var w = new BinaryWriter(s);
  foreach(double d in new[]{12d,34d,0d,0d,0d,1d,10d,5d,0d,Math.PI,0d}) w.Write(d);
  w.Write(0); return s.ToArray();
}
var color=new ProxySubentColor{ColorIndex=7};
var polygon=new ProxyPolygon();
var raw=Stream((14,BitConverter.GetBytes(7)),(44,Ellipse()),(51,BitConverter.GetBytes(0x020000FFu)),(7,new byte[4]));
var recovered=RawProxyRecovery.Recover(raw,new IProxyGeometry[]{color,polygon},1);
Check(recovered.Count==4 && ReferenceEquals(recovered[0],color) && recovered[1] is ProxyEllipseGraphic && recovered[2] is ProxyRawAttribute51 && ReferenceEquals(recovered[3],polygon),"Порядок и кратность команд нарушены");
var ellipse=(ProxyEllipseGraphic)recovered[1];
Check(ellipse.Center.X==12 && ellipse.Center.Y==34 && ellipse.MajorRadius==10 && ellipse.MinorRadius==5 && ellipse.EndParameter==Math.PI,"Параметры эллипса изменены");
Check(((ProxyRawAttribute51)recovered[2]).RawValue==0x020000FF,"Исходное значение атрибута 51 изменено");
Reject(()=>RawProxyRecovery.Recover(Stream((99,Array.Empty<byte>())),Array.Empty<IProxyGeometry>(),1),"Неизвестная команда пропущена");
Reject(()=>RawProxyRecovery.Recover(Stream((14,new byte[4])),Array.Empty<IProxyGeometry>(),1),"Потерянная известная команда пропущена");
Reject(()=>RawProxyRecovery.Recover(Stream(),new IProxyGeometry[]{color},1),"Лишняя декодированная команда пропущена");
Reject(()=>RawProxyRecovery.Recover(raw,new IProxyGeometry[]{polygon,color},1),"Несовпадение порядка пропущено");
Reject(()=>RawProxyRecovery.Recover(raw.Concat(new byte[]{0}).ToArray(),new IProxyGeometry[]{color,polygon},1),"Остаточные байты пропущены");
Reject(()=>RawProxyRecovery.Recover(Stream((44,new byte[92])),Array.Empty<IProxyGeometry>(),1),"Нулевые оси эллипса приняты");
var shellData=new byte[24*3+8+16]; BitConverter.GetBytes(3).CopyTo(shellData,0); BitConverter.GetBytes(4).CopyTo(shellData,76); BitConverter.GetBytes(-3).CopyTo(shellData,80);
Reject(()=>RawProxyRecovery.Recover(Stream((9,shellData)),new IProxyGeometry[]{new ProxyShell()},1),"Отрицательный счётчик отверстия пропущен");
var fixtureDirectory = Environment.GetEnvironmentVariable("DXF_CANVAS_PRIVATE_DWG_FIXTURES");
if(!string.IsNullOrEmpty(fixtureDirectory)) {
  int ellipses=0;
  foreach(var path in Directory.GetFiles(fixtureDirectory,"*.dwg")) {
    var originalHash=SHA256.HashData(File.ReadAllBytes(path));
    using var reader=new DwgReader(path);
    reader.Configuration=new DwgReaderConfiguration{Failsafe=false,KeepUnknownEntities=true,KeepUnknownNonGraphicalObjects=true,IgnoreProxyGraphics=false};
    var doc=reader.Read();
    var before=doc.BlockRecords.SelectMany(b=>b.Entities).ToDictionary(e=>e.Handle);
    var result=RawProxyRecovery.Augment(reader,doc);
    Check(before.Count==doc.BlockRecords.Sum(b=>b.Entities.Count) && doc.BlockRecords.SelectMany(b=>b.Entities).All(e=>ReferenceEquals(before[e.Handle],e)),"Исходные объекты изменены");
    Console.WriteLine(Path.GetFileName(path)+": "+result);
    ellipses+=result.RecoveredEllipses;
    Check(originalHash.SequenceEqual(SHA256.HashData(File.ReadAllBytes(path))),"Исходный DWG изменён");
  }
  Check(ellipses==2,"Не восстановлены две исходные эллиптические дуги");
}
Console.WriteLine($"PASS {passed}");
