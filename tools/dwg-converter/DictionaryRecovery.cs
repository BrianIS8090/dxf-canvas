using System.Collections;
using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using System.Reflection;
using System.Text;
using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.IO;
using ACadSharp.Objects;

// Обход только подтверждённого дефекта ключей MLINESTYLE в ACadSharp 3.8.35.
internal static class DictionaryRecovery
{
  private const BindingFlags Fields = BindingFlags.Instance | BindingFlags.NonPublic;
  private const string Prefix = "Error when trying to add the entry ";

  internal static CadDocument Read(string source, NotificationEventHandler notify, Action<DwgReader, CadDocument>? afterRead = null) => Read(source, notify, false, afterRead);

  internal static CadDocument ReadDxf(string source, NotificationEventHandler notify) => Read(source, notify, true);

  // Исправляет только новый DXF: writer 3.8.35 ошибочно дописывает книгу к полному имени цвета.
  internal static int FixWrittenBookColors(string destination, CadDocument document)
  {
    if (!document.RootDictionary.TryGetEntry<CadDictionary>(CadDictionary.AcadColor, out var dictionary)) return 0;
    var colors = dictionary.OfType<BookColor>().ToDictionary(c => c.Handle);
    if (colors.Count == 0) return 0;
    if (typeof(DwgReader).Assembly.GetName().Version != new Version(3, 8, 35, 0))
      throw new IOException("Исправление имён цветов проверено только для ACadSharp 3.8.35.");
    var temporary = destination + ".book-colors-" + Guid.NewGuid().ToString("N");
    var seen = new HashSet<ulong>();
    var count = 0;
    try
    {
      using (var input = File.OpenRead(destination))
      using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write))
      {
        var previous = -1;
        var current = input.ReadByte();
        while (current != -1 && current != '\n') { previous = current; current = input.ReadByte(); }
        if (current != '\n') throw new IOException("Новый DXF не содержит текстовых пар код/значение.");
        var newline = previous == '\r' ? "\r\n" : "\n";
        input.Position = 0;
        // Latin1 сохраняет каждый исходный байт, включая текст в UTF-8, без перекодирования.
        using var reader = new StreamReader(input, Encoding.Latin1, false);
        using var writer = new StreamWriter(output, Encoding.Latin1) { NewLine = newline };
        var isBookColor = false;
        ulong handle = 0;
        string? codeLine;
        while ((codeLine = reader.ReadLine()) != null)
        {
          var value = reader.ReadLine() ?? throw new IOException("Оборванная пара код/значение нового DXF.");
          var codeText = codeLine.TrimStart('\u00ef', '\u00bb', '\u00bf');
          if (!int.TryParse(codeText, NumberStyles.Integer, CultureInfo.InvariantCulture, out var code))
            throw new IOException("Некорректный код нового текстового DXF.");
          if (code == 0) { isBookColor = value == "DBCOLOR"; handle = 0; }
          if (isBookColor && code == 5 && !ulong.TryParse(value, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out handle))
            throw new IOException("Некорректный идентификатор DBCOLOR.");
          if (isBookColor && code == 430)
          {
            if (!colors.TryGetValue(handle, out var color) || !seen.Add(handle)
              || string.IsNullOrEmpty(color.BookName) || string.IsNullOrEmpty(color.ColorName)
              || color.BookName.Contains('$') || color.ColorName.Contains('$'))
              throw new IOException("DBCOLOR не соответствует исходной записи или имеет неподдерживаемое имя.");
            var expected = Encoding.Latin1.GetString(Encoding.UTF8.GetBytes(color.Name));
            var malformed = Encoding.Latin1.GetString(Encoding.UTF8.GetBytes($"{color.Name}${color.BookName}"));
            if (value == malformed) { value = expected; count++; }
            else if (value != expected) throw new IOException("Имя DBCOLOR отличается от точно известного дефекта writer 3.8.35.");
          }
          writer.WriteLine(codeLine);
          writer.WriteLine(value);
        }
      }
      if (seen.Count != colors.Count) throw new IOException("В новом DXF отсутствуют исходные записи DBCOLOR.");
      File.Move(temporary, destination, true);
      return count;
    }
    finally
    {
      if (File.Exists(temporary)) File.Delete(temporary);
    }
  }

  private static CadDocument Read(string source, NotificationEventHandler notify, bool dxf, Action<DwgReader, CadDocument>? afterRead = null)
  {
    var repairs = new List<Repair>();
    object? reader = null;
    void OnNotification(object sender, NotificationEventArgs message)
    {
      if (message.NotificationType == NotificationType.Error)
      {
        if (reader != null && TryCapture(reader, message, out var repair)
          && repairs.All(r => r.Dictionary.Handle != repair.Dictionary.Handle))
        {
          repairs.Add(repair);
          return;
        }
        throw new IOException(message.Message, message.Exception);
      }
      notify(sender, message);
    }
    using (var lifetime = dxf
      ? (IDisposable)(reader = new DxfReader(source, OnNotification) { Configuration = new DxfReaderConfiguration
      {
        Failsafe = false,
        KeepUnknownEntities = true,
        KeepUnknownNonGraphicalObjects = true,
        IgnoreProxyGraphics = false
      } })
      : (IDisposable)(reader = new DwgReader(source, OnNotification) { Configuration = new DwgReaderConfiguration
      {
        Failsafe = false,
        KeepUnknownEntities = true,
        KeepUnknownNonGraphicalObjects = true,
        IgnoreProxyGraphics = false
      } }))
    {
      var document = reader is DxfReader dxfReader ? dxfReader.Read() : ((DwgReader)reader).Read();
      if (repairs.Count == 0)
      {
        if (reader is DwgReader dwgReader)
          afterRead?.Invoke(dwgReader, document);
        return document;
      }
      var before = document.BlockRecords.SelectMany(b => b.Entities).ToDictionary(e => e.Handle);
      foreach (var repair in repairs)
        Restore(document, repair);
      RestoreStyleReferences(reader, document, repairs);
      var after = document.BlockRecords.SelectMany(b => b.Entities).ToDictionary(e => e.Handle);
      if (before.Count != after.Count || before.Any(e => !after.TryGetValue(e.Key, out var value) || !ReferenceEquals(e.Value, value)))
        throw new IOException("Восстановление словаря изменило исходные сущности.");
      foreach (var repair in repairs)
      {
        var details = string.Join(", ", repair.Entries.Select(e => $"{e.Key} → {e.Handle:X}"));
        notify(reader, new NotificationEventArgs($"Восстановлены исходные ключи словаря {repair.Dictionary.Handle:X}: {details}; обе записи и их идентификаторы сохранены.", NotificationType.Warning));
      }
      if (reader is DwgReader repairedDwgReader)
        afterRead?.Invoke(repairedDwgReader, document);
      return document;
    }
  }

  private sealed record Entry(string Key, ulong Handle, MLineStyle Value);
  private sealed record Repair(CadDictionary Dictionary, Entry[] Entries);

  private static bool TryCapture(object reader, NotificationEventArgs message, out Repair repair)
  {
    repair = null!;
    if (message.Exception is not ArgumentException { ParamName: null, InnerException: null }
      || !message.Message.StartsWith(Prefix, StringComparison.Ordinal))
      return false;
    var separator = message.Message.LastIndexOf('|');
    if (separator < 0 || !ulong.TryParse(message.Message.AsSpan(separator + 1), NumberStyles.None, CultureInfo.InvariantCulture, out var handle))
      return false;
    var builder = GetBuilder(reader);
    var dictionaries = (IDictionary)GetBuilderField(builder, "dictionaryTemplates");
    var objects = (Dictionary<ulong, CadObject>)GetBuilderField(builder, "cadObjects");
    if (!dictionaries.Contains(handle) || !objects.TryGetValue(handle, out var dictionaryObject)
      || dictionaryObject is not CadDictionary dictionary)
      return false;
    var template = dictionaries[handle]!;
    var original = GetOriginalEntries(template);
    // Два разных объекта одного типа, два разных исходных ключа и одна уже добавленная запись.
    if (original.Count != 2 || dictionary.Count() != 1
      || original.Keys.Distinct(StringComparer.OrdinalIgnoreCase).Count() != 2)
      return false;
    var entries = new List<Entry>();
    foreach (var pair in original)
    {
      if (!pair.Value.HasValue || !objects.TryGetValue(pair.Value.Value, out var value) || value is not MLineStyle style)
        return false;
      entries.Add(new Entry(pair.Key, pair.Value.Value, style));
    }
    if (entries[0].Handle == entries[1].Handle
      || !StringComparer.OrdinalIgnoreCase.Equals(entries[0].Value.Name, entries[1].Value.Name)
      || message.Message != $"{Prefix}{entries[1].Value.Name} to {dictionary.Name}|{handle}"
      || !ReferenceEquals(dictionary.First(), entries[0].Value)
      || !dictionary.ContainsKey(entries[1].Value.Name)
      || entries[1].Value.Owner != null)
      return false;
    repair = new Repair(dictionary, entries.ToArray());
    return true;
  }

  private static void Restore(CadDocument document, Repair repair)
  {
    var dictionary = repair.Dictionary;
    var first = repair.Entries[0];
    var second = repair.Entries[1];
    if (!ReferenceEquals(document.GetCadObject(dictionary.Handle), dictionary)
      || !ReferenceEquals(document.GetCadObject(first.Handle), first.Value)
      || document.GetCadObject(second.Handle) != null
      || first.Value.Handle != first.Handle || second.Value.Handle != second.Handle
      || dictionary.Count() != 1 || !ReferenceEquals(dictionary.First(), first.Value))
      throw new IOException("Состав повреждённого словаря изменился до восстановления.");
    // Remove сбрасывает handle и связи; меняем только ключ во внутренней таблице.
    var entries = GetDictionaryEntries(dictionary);
    entries.Clear();
    entries.Add(first.Key, first.Value);
    dictionary.Add(second.Key, second.Value);
    foreach (var entry in repair.Entries)
      if (!ReferenceEquals(dictionary[entry.Key], entry.Value)
        || entry.Value.Handle != entry.Handle || !ReferenceEquals(entry.Value.Owner, dictionary)
        || !ReferenceEquals(document.GetCadObject(entry.Handle), entry.Value))
        throw new IOException("Не удалось сохранить обе записи словаря и их исходные идентификаторы.");
  }

  private static void RestoreStyleReferences(object reader, CadDocument document, List<Repair> repairs)
  {
    var styles = repairs.SelectMany(r => r.Entries).ToDictionary(e => e.Handle, e => e.Value);
    var templates = (IDictionary)GetBuilderField(GetBuilder(reader), "templatesMap");
    foreach (DictionaryEntry pair in templates)
    {
      if (!TryGetMLineStyleHandle(pair.Value!, out var styleHandle) || !styles.TryGetValue(styleHandle, out var style))
        continue;
      var line = document.GetCadObject<MLine>((ulong)pair.Key)
        ?? throw new IOException("Потеряна сущность со ссылкой на восстанавливаемый стиль.");
      // Публичный setter объединяет стили по имени; восстанавливаем исходную ссылку по handle.
      SetMLineStyle(line, style);
      if (!ReferenceEquals(line.Style, style))
        throw new IOException("Не восстановлена исходная ссылка MLINE на стиль.");
    }
  }

  [DynamicDependency("_builder", typeof(DwgReader))]
  [DynamicDependency("_builder", typeof(DxfReader))]
  private static object GetBuilder(object reader)
  {
    if (typeof(DwgReader).Assembly.GetName().Version != new Version(3, 8, 35, 0))
      throw new IOException("Восстановление словарей проверено только для ACadSharp 3.8.35.");
    return (reader is DxfReader ? typeof(DxfReader) : typeof(DwgReader)).GetField("_builder", Fields)?.GetValue(reader)
      ?? throw new IOException("Недоступен исходный построитель DWG.");
  }

  [DynamicDependency("cadObjects", "ACadSharp.IO.CadDocumentBuilder", "ACadSharp")]
  [DynamicDependency("dictionaryTemplates", "ACadSharp.IO.CadDocumentBuilder", "ACadSharp")]
  [DynamicDependency("templatesMap", "ACadSharp.IO.CadDocumentBuilder", "ACadSharp")]
  [UnconditionalSuppressMessage("Trimming", "IL2075", Justification = "Три поля явно сохранены DynamicDependency; версия библиотеки проверяется.")]
  private static object GetBuilderField(object builder, string name) =>
    builder.GetType().BaseType!.GetField(name, Fields)?.GetValue(builder)
      ?? throw new IOException($"Недоступны исходные данные DWG: {name}.");

  [DynamicDependency("Entries", "ACadSharp.IO.Templates.CadDictionaryTemplate", "ACadSharp")]
  [UnconditionalSuppressMessage("Trimming", "IL2075", Justification = "Свойство Entries явно сохранено DynamicDependency.")]
  private static Dictionary<string, ulong?> GetOriginalEntries(object template) =>
    template.GetType().GetProperty("Entries")?.GetValue(template) as Dictionary<string, ulong?>
      ?? throw new IOException("Недоступны исходные ключи словаря DWG.");

  [DynamicDependency("_entries", typeof(CadDictionary))]
  private static Dictionary<string, NonGraphicalObject> GetDictionaryEntries(CadDictionary dictionary) =>
    (Dictionary<string, NonGraphicalObject>)(typeof(CadDictionary).GetField("_entries", Fields)?.GetValue(dictionary)
      ?? throw new IOException("Недоступны ключи восстанавливаемого словаря."));

  [DynamicDependency("MLineStyleHandle", "ACadSharp.IO.Templates.CadMLineTemplate", "ACadSharp")]
  [UnconditionalSuppressMessage("Trimming", "IL2075", Justification = "Свойство MLineStyleHandle явно сохранено DynamicDependency.")]
  private static bool TryGetMLineStyleHandle(object template, out ulong handle)
  {
    handle = 0;
    if (template.GetType().FullName != "ACadSharp.IO.Templates.CadMLineTemplate")
      return false;
    var value = template.GetType().GetProperty("MLineStyleHandle")?.GetValue(template);
    if (value is not ulong result)
      throw new IOException("Недоступна исходная ссылка MLINE на стиль.");
    handle = result;
    return true;
  }

  [DynamicDependency("_style", typeof(MLine))]
  private static void SetMLineStyle(MLine line, MLineStyle style)
  {
    var field = typeof(MLine).GetField("_style", Fields)
      ?? throw new IOException("Недоступна ссылка MLINE на стиль.");
    field.SetValue(line, style);
  }
}
