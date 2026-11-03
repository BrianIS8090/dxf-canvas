using System.Reflection;
using System.Text;
using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.IO;
using ACadSharp.Objects;
using CSMath;

Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
var directory = Path.Combine(args.Length > 0 ? Path.GetFullPath(args[0]) : Path.GetTempPath(), "dictionary-recovery-tests-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(directory);

void Assert(bool condition, string message)
{
  if (!condition) throw new Exception(message);
}

void FatalNotification(object sender, NotificationEventArgs message)
{
  if (message.NotificationType == NotificationType.Error)
    throw new IOException(message.Message, message.Exception);
}

var source = new CadDocument(ACadVersion.AC1032);
var styles = source.RootDictionary.GetEntry<CadDictionary>(CadDictionary.AcadMLineStyle);
styles.Clear();
var first = new MLineStyle("STANDARD") { Description = "Первый стиль" };
var second = new MLineStyle("STANDARD") { Description = "Второй стиль" };
styles.Add("AUDIT_E_SYNTHETIC", first);
styles.Add("Standard", second);
source.Entities.Add(new Line(XYZ.Zero, XYZ.AxisX));
var references = new List<(ulong Entity, ulong Style)>();
foreach (var style in new[] { first, second })
{
  var line = new MLine();
  line.Vertices.Add(new MLine.Vertex { Position = XYZ.Zero, Direction = XYZ.AxisX, Miter = XYZ.AxisY });
  line.Vertices.Add(new MLine.Vertex { Position = XYZ.AxisX, Direction = XYZ.AxisX, Miter = XYZ.AxisY });
  source.Entities.Add(line);
  // Сам читатель ошибочно объединяет ссылки по имени; тест задаёт исходные разные ссылки.
  typeof(MLine).GetField("_style", BindingFlags.Instance | BindingFlags.NonPublic)!.SetValue(line, style);
  references.Add((line.Handle, style.Handle));
}
var originalEntities = source.Entities.Select(e => e.Handle).Order().ToArray();
var path = Path.Combine(directory, "duplicate-styles.dwg");
using (var output = File.Create(path)) DwgWriter.Write(output, source);

var strictFailed = false;
try
{
  DwgReader.Read(path, new DwgReaderConfiguration { Failsafe = false, KeepUnknownEntities = true, IgnoreProxyGraphics = false }, FatalNotification);
}
catch (IOException ex) when (ex.Message.StartsWith("Error when trying to add the entry STANDARD", StringComparison.Ordinal))
{
  strictFailed = true;
}
Assert(strictFailed, "Синтетический исходник не воспроизвёл ошибку библиотеки.");
Console.WriteLine("PASS: исходная ошибка STANDARD воспроизведена.");

void Check(CadDocument document)
{
  var dictionary = document.RootDictionary.GetEntry<CadDictionary>(CadDictionary.AcadMLineStyle);
  Assert(dictionary.Count() == 2, "Потерян один из стилей.");
  Assert(dictionary["AUDIT_E_SYNTHETIC"].Handle == first.Handle && dictionary["Standard"].Handle == second.Handle, "Изменились исходные ключи или идентификаторы.");
  Assert(dictionary.All(e => e.Name == "STANDARD"), "Изменены исходные имена записей.");
  Assert(((MLineStyle)dictionary["AUDIT_E_SYNTHETIC"]).Description == first.Description, "Изменена первая запись.");
  Assert(((MLineStyle)dictionary["Standard"]).Description == second.Description, "Изменена вторая запись.");
  Assert(document.Entities.Select(e => e.Handle).Order().SequenceEqual(originalEntities), "Потеряны или заменены сущности.");
  foreach (var pair in references)
    Assert(document.GetCadObject<MLine>(pair.Entity)?.Style.Handle == pair.Style, "Подменена исходная ссылка MLINE.");
}

var warnings = new List<string>();
var callbackCount = 0;
var recovered = DictionaryRecovery.Read(path, (s, n) =>
{
  FatalNotification(s, n);
  warnings.Add(n.Message);
}, (reader, document) => { callbackCount++; Check(document); });
Check(recovered);
Assert(callbackCount == 1, "Обработчик исходной графики не вызван после восстановления.");
Assert(warnings.Count(m => m.StartsWith("Восстановлены исходные ключи", StringComparison.Ordinal)) == 1, "Нет точного отчёта о восстановлении.");
Console.WriteLine("PASS: ключи, две записи, исходные имена, сущности и обе ссылки MLINE сохранены.");

var dxf = Path.ChangeExtension(path, ".dxf");
using (var output = File.Create(dxf)) DxfWriter.Write(output, recovered, false, new DxfWriterConfiguration(), FatalNotification);
Assert(File.ReadAllText(dxf).Contains("AUDIT_E_SYNTHETIC", StringComparison.Ordinal), "Исходный ключ отсутствует в записанном DXF.");
Check(DictionaryRecovery.ReadDxf(dxf, FatalNotification));
Console.WriteLine("PASS: проверка DWG → DXF → чтение DXF сохраняет обе записи и ссылки.");

styles.Add("THIRD_ALIAS", new MLineStyle("STANDARD"));
var unsupported = Path.Combine(directory, "three-colliding-styles.dwg");
using (var output = File.Create(unsupported)) DwgWriter.Write(output, source);
var unsupportedFailed = false;
try { DictionaryRecovery.Read(unsupported, (_, _) => { }); }
catch (IOException ex) when (ex.Message.StartsWith("Error when trying to add the entry STANDARD", StringComparison.Ordinal)) { unsupportedFailed = true; }
Assert(unsupportedFailed, "Неподтверждённый конфликт из трёх записей был скрыт.");

// Произвольная ошибка не должна проходить даже начальную проверку восстановления.
var capture = typeof(DictionaryRecovery).GetMethod("TryCapture", BindingFlags.Static | BindingFlags.NonPublic)!;
var arbitraryArguments = new object?[] { new object(), new NotificationEventArgs("Произвольная ошибка чтения", NotificationType.Error, new ArgumentException("Ошибка")), null };
Assert(!(bool)capture.Invoke(null, arbitraryArguments)!, "Произвольная ошибка ошибочно признана восстановимой.");
Console.WriteLine("PASS: произвольная ошибка и неподтверждённый конфликт отклоняются.");

var normal = Path.Combine(directory, "normal.dwg");
using (var output = File.Create(normal)) DwgWriter.Write(output, new CadDocument(ACadVersion.AC1032));
DictionaryRecovery.Read(normal, FatalNotification, (_, _) => callbackCount++);
Assert(callbackCount == 2, "Обработчик исходной графики не вызван для исправного DWG.");
Console.WriteLine("PASS: обычный DWG читается, обработчик до закрытия читателя вызван.");

var colorDocument = new CadDocument(ACadVersion.AC1032);
var colorDictionary = colorDocument.RootDictionary.GetEntry<CadDictionary>(CadDictionary.AcadColor);
var expectedColors = new[] {
  new BookColor("Синий", "Книга") { Color = new Color(14, 30, 180) },
  new BookColor("Красный", "Книга") { Color = new Color(180, 30, 14) }
};
foreach (var color in expectedColors) colorDictionary.Add(color);
var colorsPath = Path.Combine(directory, "book-colors.dxf");
using (var output = File.Create(colorsPath)) DxfWriter.Write(output, colorDocument, false);
var colorFailure = false;
try { DictionaryRecovery.ReadDxf(colorsPath, FatalNotification); }
catch (IOException ex) when (ex.Message.Contains("Error when trying to add the entry", StringComparison.Ordinal)) { colorFailure = true; }
Assert(colorFailure, "Не воспроизведён дефект записи имён DBCOLOR.");
Assert(DictionaryRecovery.FixWrittenBookColors(colorsPath, colorDocument) == 2, "Не исправлены ровно две записи DBCOLOR.");
var fixedColors = DictionaryRecovery.ReadDxf(colorsPath, FatalNotification);
foreach (var expected in expectedColors)
{
  var actual = fixedColors.GetCadObject<BookColor>(expected.Handle);
  Assert(actual != null && actual.Name == expected.Name && actual.Color.Equals(expected.Color), "Изменились имя, RGB или идентификатор DBCOLOR.");
  Assert(fixedColors.RootDictionary.GetEntry<CadDictionary>(CadDictionary.AcadColor)[expected.Name].Handle == expected.Handle, "Изменился исходный ключ цветовой книги.");
}
Assert(DictionaryRecovery.FixWrittenBookColors(colorsPath, colorDocument) == 0, "Повторная проверка исправного DXF меняет данные.");
var damagedColorText = File.ReadAllText(colorsPath).Replace("Книга$Синий", "Книга$Непредусмотренный", StringComparison.Ordinal);
File.WriteAllText(colorsPath, damagedColorText);
var damagedBefore = File.ReadAllBytes(colorsPath);
var damagedFailed = false;
try { DictionaryRecovery.FixWrittenBookColors(colorsPath, colorDocument); }
catch (IOException) { damagedFailed = true; }
Assert(damagedFailed && File.ReadAllBytes(colorsPath).SequenceEqual(damagedBefore), "Произвольное имя исправлено без доказательства или изменён файл при отказе.");
Console.WriteLine("PASS: DBCOLOR исправляется только по точному дефекту, имена/ключи/RGB/handles сохранены; неизвестное имя отклонено.");
Console.WriteLine($"Все проверки пройдены. Синтетические файлы: {directory}");
