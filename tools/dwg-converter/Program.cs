using System.Text;
using System.Text.Json;
using ACadSharp.IO;
using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Tables;
using CSMath;

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
  var configuration = new DwgReaderConfiguration
  {
    Failsafe = false,
    KeepUnknownEntities = true,
    IgnoreProxyGraphics = false
  };
  var document = DwgReader.Read(source, configuration, Notify);
  // CreateNew не позволяет конвертеру затереть существующий файл.
  using (var stream = new FileStream(destination, FileMode.CreateNew, FileAccess.Write))
  {
    DxfWriter.Write(stream, document, false, new DxfWriterConfiguration(), Notify);
  }
  // Явная запись JSON не требует отражения и совместима с нативной сборкой.
  using var writer = new Utf8JsonWriter(Console.OpenStandardOutput());
  writer.WriteStartObject();
  writer.WriteString("engine", "ACadSharp 3.7.1");
  writer.WriteString("version", document.Header.Version.ToString());
  writer.WriteNumber("layers", document.Layers.Count);
  writer.WriteNumber("entities", document.Entities.Count);
  writer.WriteNumber("warning_count", warningCount);
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
