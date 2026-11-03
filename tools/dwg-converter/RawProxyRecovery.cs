using System.Collections;
using System.Diagnostics.CodeAnalysis;
using System.Reflection;
using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Entities.AecEntities;
using ACadSharp.Entities.ProxyGraphics;
using ACadSharp.IO;
using CSMath;

internal sealed record RawProxyRecoveryResult(int Entities, int Commands, int RecoveredEllipses, int Attribute51Commands);

// Проверка исходного потока до потери неизвестных команд внутри ACadSharp 3.8.35.
internal static class RawProxyRecovery
{
  [DynamicDependency(DynamicallyAccessedMemberTypes.NonPublicFields, typeof(DwgReader))]
  [DynamicDependency(DynamicallyAccessedMemberTypes.NonPublicFields, "ACadSharp.IO.CadDocumentBuilder", "ACadSharp")]
  [DynamicDependency(DynamicallyAccessedMemberTypes.PublicProperties, "ACadSharp.IO.Templates.CadEntityTemplate", "ACadSharp")]
  internal static RawProxyRecoveryResult Augment(DwgReader reader, CadDocument document)
  {
    var assembly = typeof(DwgReader).Assembly;
    var builder = typeof(DwgReader).GetField("_builder", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(reader)
      ?? throw new IOException("Недоступен построитель ACadSharp для проверки исходной прокси-графики.");
    var builderType = assembly.GetType("ACadSharp.IO.CadDocumentBuilder", true)!;
    var templates = builderType.GetField("templatesMap", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(builder) as IDictionary
      ?? throw new IOException("Недоступны исходные шаблоны прокси-графики.");
    var templateType = assembly.GetType("ACadSharp.IO.Templates.CadEntityTemplate", true)!;
    var rawProperty = templateType.GetProperty("ProxyGraphics")
      ?? throw new IOException("Недоступен исходный массив прокси-графики.");
    int count = 0, commands = 0, ellipses = 0, attributes = 0;
    foreach (var entity in document.BlockRecords.SelectMany(b => b.Entities))
    {
      if (entity is not (UnknownEntity or Wall or TableEntity or MultiLeader))
        continue;
      if (!templates.Contains(entity.Handle) || !templateType.IsInstanceOfType(templates[entity.Handle]))
        throw new IOException($"Нет исходного шаблона прокси-графики {entity.Handle:X}.");
      if (rawProperty.GetValue(templates[entity.Handle]) is not byte[] raw || raw.Length == 0)
      {
        if (entity.ProxyGeometries.Count > 0)
          throw new IOException($"Есть декодированная графика без исходных команд {entity.Handle:X}.");
        continue;
      }
      var recovered = Recover(raw, entity.ProxyGeometries, entity.Handle);
      entity.ProxyGeometries.Clear();
      entity.ProxyGeometries.AddRange(recovered);
      count++;
      commands += recovered.Count;
      ellipses += recovered.Count(g => g is ProxyEllipseGraphic);
      attributes += recovered.Count(g => g is ProxyRawAttribute51);
    }
    return new(count, commands, ellipses, attributes);
  }

  internal static List<IProxyGeometry> Recover(byte[] raw, IReadOnlyList<IProxyGeometry> decoded, ulong handle)
  {
    if (raw.Length < 8)
      throw Invalid(handle, "обрезан заголовок");
    int count = BitConverter.ToInt32(raw, 4), position = 8, decodedIndex = 0;
    if (count < 0 || count > (raw.Length - 8) / 8)
      throw Invalid(handle, "недопустимое число команд");
    var recovered = new List<IProxyGeometry>(count);
    for (int i = 0; i < count; i++)
    {
      if (position > raw.Length - 8)
        throw Invalid(handle, "обрезан заголовок команды");
      int size = BitConverter.ToInt32(raw, position), type = BitConverter.ToInt32(raw, position + 4);
      if (size < 8 || size > raw.Length - position)
        throw Invalid(handle, $"недопустимая длина команды {type}");
      var payload = raw.AsSpan(position + 8, size - 8);
      if (type == 44)
        recovered.Add(ReadEllipse(payload, handle));
      else if (type == 51)
      {
        if (payload.Length != 4)
          throw Invalid(handle, "недопустимая длина атрибута 51");
        recovered.Add(new ProxyRawAttribute51 { RawValue = BitConverter.ToUInt32(payload) });
      }
      else
      {
        // Ни неизвестный тип, ни несовпадение порядка не могут исчезнуть бесследно.
        if (decodedIndex >= decoded.Count || (int)decoded[decodedIndex].GraphicsType != type)
          throw Invalid(handle, $"неподдержанная или потерянная команда {type}, позиция {i}");
        if (type == 9)
          CheckShell(payload, handle);
        recovered.Add(decoded[decodedIndex++]);
      }
      position += size;
    }
    if (position != raw.Length || decodedIndex != decoded.Count)
      throw Invalid(handle, "число команд или конечная позиция не совпадает с исходным потоком");
    return recovered;
  }

  private static ProxyEllipseGraphic ReadEllipse(ReadOnlySpan<byte> data, ulong handle)
  {
    // Формат подтверждён независимым декодером ezdxf, proxygraphic.py::elliptic_arc.
    // В исследованных файлах последняя четверка байтов — нулевое дополнение.
    if (data.Length != 92 || BitConverter.ToUInt32(data[88..]) != 0)
      throw Invalid(handle, "неподдержанный формат эллиптической дуги");
    var values = new double[11];
    for (int i = 0; i < values.Length; i++)
    {
      values[i] = BitConverter.ToDouble(data.Slice(i * 8, 8));
      if (!double.IsFinite(values[i]))
        throw Invalid(handle, "нечисловой параметр эллиптической дуги");
    }
    if (values[6] <= 0 || values[7] <= 0 || values[7] > values[6]
      || values[3] * values[3] + values[4] * values[4] + values[5] * values[5] < 1e-20)
      throw Invalid(handle, "недопустимые оси эллиптической дуги");
    return new ProxyEllipseGraphic
    {
      Center = new XYZ(values[0], values[1], values[2]),
      Normal = new XYZ(values[3], values[4], values[5]),
      MajorRadius = values[6], MinorRadius = values[7],
      StartParameter = values[8], EndParameter = values[9], MajorAngle = values[10]
    };
  }

  private static void CheckShell(ReadOnlySpan<byte> data, ulong handle)
  {
    if (data.Length < 8)
      throw Invalid(handle, "обрезана оболочка");
    int vertices = BitConverter.ToInt32(data);
    if (vertices < 0 || vertices > (data.Length - 8) / 24)
      throw Invalid(handle, "неверное число вершин оболочки");
    int position = 4 + vertices * 24;
    int indices = BitConverter.ToInt32(data[position..]);
    position += 4;
    if (indices < 0 || indices > (data.Length - position) / 4)
      throw Invalid(handle, "неверный размер граней оболочки");
    for (int consumed = 0; consumed < indices;)
    {
      int faceCount = BitConverter.ToInt32(data[position..]);
      if (faceCount < 0)
        throw Invalid(handle, "оболочка содержит отверстие, знак которого теряет ACadSharp");
      if (faceCount < 3 || faceCount > indices - consumed - 1)
        throw Invalid(handle, "неверное число вершин грани оболочки");
      consumed += faceCount + 1;
      position += (faceCount + 1) * 4;
    }
  }

  private static IOException Invalid(ulong handle, string reason) =>
    new($"Исходная прокси-графика {handle:X}: {reason}.");
}
