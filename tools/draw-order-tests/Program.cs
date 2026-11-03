using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Entities.AecEntities;
using ACadSharp.Entities.ProxyGraphics;
using ACadSharp.IO;
using ACadSharp.Tables;
using System.Security.Cryptography;
using System.Text;

Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
int checks = 0;
void Check(bool condition, string message) { if (!condition) throw new Exception(message); checks++; }
DrawOrderRecovery Capture(CadDocument doc) => DrawOrderRecovery.Capture(doc);
void Apply(DrawOrderRecovery snapshot, CadDocument doc, ProxyConversionReport report) => snapshot.Apply(doc, report);
Wall MakeWall() {
  var wall = new Wall();
  wall.ProxyGeometries.Add(new ProxyPolyline { Points = [new(0,0,0), new(1,0,0)] });
  return wall;
}
void Fixture(bool explicitOrder, bool equalKeys, bool zeroKey = false, bool nested = false) {
  var doc = new CadDocument();
  var first = MakeWall(); var middle = new Line(new(3,0,0),new(4,0,0)); var last = MakeWall();
  var block = nested ? new BlockRecord("Повторяемый") : (BlockRecord)doc.Entities.Owner;
  if (nested) {
    doc.BlockRecords.Add(block);
    doc.Entities.Add(new Insert(block));
    doc.Entities.Add(new Insert(block) { InsertPoint = new(10,0,0) });
  }
  block.Entities.Add(first); block.Entities.Add(middle); block.Entities.Add(last);
  var firstHandle = first.Handle; var middleHandle = middle.Handle; var lastHandle = last.Handle;
  if (explicitOrder) {
    var table = block.CreateSortEntitiesTable();
    table.Add(first, zeroKey ? 0 : equalKeys ? middleHandle : lastHandle);
    table.Add(last, equalKeys ? middleHandle : firstHandle);
    table.Add(middle, middleHandle);
  }
  var before = block.Entities.Select(e => e.Handle).ToArray();
  var keys = block.Entities.ToDictionary(e => e.Handle, e => block.SortEntitiesTable?.GetSorterHandle(e) ?? e.Handle);
  var snapshot = Capture(doc);
  var report = ProxyConversion.Convert(doc);
  var mapping = report.Mappings.ToDictionary(m => m.SourceHandle, m => m.InsertHandle);
  Apply(snapshot, doc, report);
  ulong Current(ulong h) => mapping.GetValueOrDefault(h, h);
  Check(block.Entities.Select(e => e.Handle).SequenceEqual(before.Select(Current)), "Изменён исходный порядок записей блока.");
  foreach (var pair in keys) Check(block.SortEntitiesTable.GetSorterHandle(doc.GetCadObject<Entity>(Current(pair.Key))) == pair.Value, "Потерян исходный числовой ключ сортировки.");
  Check(block.SortEntitiesTable.All(s => s.Entity.Owner == block && s.Entity.Handle != 0), "Таблица содержит удалённый объект или неверного владельца.");
  using var output = new MemoryStream(); DxfWriter.Write(output, doc, false, new DxfWriterConfiguration());
  using var input = new MemoryStream(output.ToArray()); var restored = DxfReader.Read(input);
  var restoredBlock = restored.GetCadObject<BlockRecord>(block.Handle);
  foreach (var pair in keys) Check(restoredBlock.SortEntitiesTable.GetSorterHandle(restored.GetCadObject<Entity>(Current(pair.Key))) == pair.Value, "DXF потерял исходный ключ сортировки.");
  Check(restoredBlock.Entities.Select(e => e.Handle).SequenceEqual(before.Select(Current)), "DXF изменил порядок записей при равных ключах.");
  snapshot.Verify(restored, report);
}
Fixture(false, false);
Fixture(true, false);
Fixture(true, true);
Fixture(true, false, true);
Fixture(true, true, false, true);
var fixtureDirectory = Environment.GetEnvironmentVariable("DXF_CANVAS_PRIVATE_DWG_FIXTURES");
if (!string.IsNullOrEmpty(fixtureDirectory)) {
  foreach (var path in Directory.GetFiles(fixtureDirectory, "*.dwg")) {
    var originalHash = SHA256.HashData(File.ReadAllBytes(path));
    var doc = DictionaryRecovery.Read(path, (_, _) => {}, (reader, drawing) => RawProxyRecovery.Augment(reader, drawing));
    var snapshot = Capture(doc);
    var report = ProxyConversion.Convert(doc);
    Apply(snapshot, doc, report);
    snapshot.Verify(doc, report);
    Check(originalHash.SequenceEqual(SHA256.HashData(File.ReadAllBytes(path))), "Исходный DWG изменён.");
    Console.WriteLine($"PASS DWG: {Path.GetFileName(path)}, замен {report.ReplacedCount}");
  }
}
Console.WriteLine($"PASS {checks}: замены, обычные сущности, явные ключи, равные ключи, запись DXF.");
