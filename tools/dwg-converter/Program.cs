using System.Text;
using System.Text.Json;
using ACadSharp.IO;
using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Tables;
using CSMath;

Console.OutputEncoding = new UTF8Encoding(false);

// Синтетический чертёж для автоматических проверок; производственные файлы не публикуются.
if (args.Length == 2 && args[0] == "--write-test-dwg")
{
  Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
  var document = new CadDocument(ACadVersion.AC1032);
  var contour = new Layer("Контур");
  document.Layers.Add(contour);
  foreach (var (a, b) in new[] {
    (new XYZ(0, 0, 0), new XYZ(160, 0, 0)),
    (new XYZ(160, 0, 0), new XYZ(160, 100, 0)),
    (new XYZ(160, 100, 0), new XYZ(0, 100, 0)),
    (new XYZ(0, 100, 0), new XYZ(0, 0, 0)) })
    document.Entities.Add(new Line(a, b) { Layer = contour });
  document.Entities.Add(new Circle { Center = new XYZ(80, 50, 0), Radius = 5, Layer = contour });
  document.Entities.Add(new TextEntity { Value = "Тест DWG", InsertPoint = new XYZ(20, 80, 0), Height = 3, Layer = contour });
  using var stream = new FileStream(args[1], FileMode.CreateNew, FileAccess.Write);
  DwgWriter.Write(stream, document, new DwgWriterConfiguration());
  return 0;
}

if (args.Length != 2)
{
  Console.Error.WriteLine("Нужны пути входного DWG и нового выходного DXF.");
  return 2;
}

try
{
  Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
  var source = Path.GetFullPath(args[0]);
  var destination = Path.GetFullPath(args[1]);
  if (String.Equals(source, destination, StringComparison.OrdinalIgnoreCase))
    throw new IOException("Исходный чертёж нельзя перезаписывать.");
  var warnings = new List<string>();
  var warningCount = 0;
  void Notify(object sender, NotificationEventArgs notification)
  {
    if (notification.NotificationType == NotificationType.None)
      return;
    if (notification.NotificationType == NotificationType.Error)
      throw new IOException(notification.Message, notification.Exception);
    warningCount++;
    if (warnings.Count < 100)
    {
      var message = $"{notification.NotificationType}: {notification.Message}";
      warnings.Add(message.Length > 1000 ? message[..1000] : message);
    }
  }
  RawProxyRecoveryResult? rawCoverage = null;
  var document = DictionaryRecovery.Read(source, Notify, (reader, drawing) => rawCoverage = RawProxyRecovery.Augment(reader, drawing));
  if (rawCoverage is { Attribute51Commands: > 0 })
  {
    warningCount++;
    if (warnings.Count == 100) warnings.RemoveAt(99);
    warnings.Insert(0, $"Сохранённая графика содержит {rawCoverage.Attribute51Commands} команд 51. Значения 0 и 0x020000FF интерпретированы как прозрачность по слою и непрозрачность по структуре данных; публичная спецификация команды не найдена. Внешний вид следует сверить с CAD.");
  }
  var originals = ConversionAudit.Snapshot(document);
  var drawOrder = DrawOrderRecovery.Capture(document);
  var conversion = ProxyConversion.Convert(document);
  drawOrder.Apply(document, conversion);
  ConversionAudit.Verify(document, originals, conversion);
  // CreateNew не позволяет конвертеру затереть существующий файл.
  using (var stream = new FileStream(destination, FileMode.CreateNew, FileAccess.Write))
  {
    DxfWriter.Write(stream, document, false, new DxfWriterConfiguration(), Notify);
  }
  DictionaryRecovery.FixWrittenBookColors(destination, document);
  var restored = ConversionAudit.VerifyWritten(destination, document, Notify);
  drawOrder.Verify(restored, conversion);
  // Явная запись JSON не требует отражения и совместима с нативной сборкой.
  using var writer = new Utf8JsonWriter(Console.OpenStandardOutput());
  writer.WriteStartObject();
  writer.WriteString("engine", "ACadSharp 3.8.35");
  writer.WriteString("version", document.Header.Version.ToString());
  writer.WriteNumber("layers", document.Layers.Count);
  writer.WriteNumber("entities", document.Entities.Count);
  writer.WriteNumber("warning_count", warningCount);
  writer.WriteStartObject("proxy_conversion");
  writer.WriteNumber("source_count", conversion.SourceCount);
  writer.WriteNumber("replaced_count", conversion.ReplacedCount);
  writer.WriteNumber("generated_count", conversion.GeneratedCount);
  writer.WriteBoolean("verified", true);
  writer.WriteNumber("duplicate_representations", 0);
  writer.WriteNumber("raw_commands", rawCoverage?.Commands ?? 0);
  writer.WriteNumber("recovered_ellipses", rawCoverage?.RecoveredEllipses ?? 0);
  writer.WriteEndObject();
  // Полный список замен хранится рядом с временным DXF для приёмочных проверок.
  using (var auditStream = new FileStream(destination + ".coverage.json", FileMode.CreateNew))
  using (var audit = new Utf8JsonWriter(auditStream))
  {
    audit.WriteStartArray();
    foreach (var mapping in conversion.Mappings)
    {
      audit.WriteStartObject();
      audit.WriteString("source_handle", mapping.SourceHandle.ToString("X"));
      audit.WriteString("source_type", mapping.SourceType);
      audit.WriteString("block", mapping.BlockName);
      audit.WriteString("insert_handle", mapping.InsertHandle.ToString("X"));
      audit.WriteNumber("generated_count", mapping.GeneratedCount);
      audit.WriteEndObject();
    }
    audit.WriteEndArray();
  }
  writer.WriteStartArray("warnings");
  foreach (var warning in warnings)
    writer.WriteStringValue(warning);
  writer.WriteEndArray();
  writer.WriteEndObject();
  writer.Flush();
  return 0;
}
catch (Exception exception)
{
  Console.Error.WriteLine(exception.ToString());
  return 1;
}
