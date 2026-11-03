using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.IO;

// Сверка охватывает определения блоков, а не только пространство модели.
public static class ConversionAudit
{
  public static Dictionary<ulong, string> Snapshot(CadDocument document)
  {
    return document.BlockRecords.SelectMany(block => block.Entities)
      .ToDictionary(entity => entity.Handle, entity => entity.GetType().Name);
  }

  public static void Verify(CadDocument document, Dictionary<ulong, string> source, ProxyConversionReport report)
  {
    var entities = document.BlockRecords.SelectMany(block => block.Entities).ToArray();
    var actual = entities.ToDictionary(entity => entity.Handle);
    var inserts = entities.OfType<Insert>().GroupBy(insert => insert.Block.Name).ToDictionary(group => group.Key, group => group.Count());
    var replaced = report.Mappings.ToDictionary(mapping => mapping.SourceHandle);
    if (report.SourceCount != source.Count || report.ReplacedCount != replaced.Count)
      throw new IOException("Некорректный отчёт покрытия объектов DWG.");
    foreach (var pair in source)
    {
      if (replaced.TryGetValue(pair.Key, out var mapping))
      {
        if (actual.ContainsKey(pair.Key))
          throw new IOException($"Объект #{pair.Key:X} представлен одновременно исходником и графикой.");
        if (!actual.TryGetValue(mapping.InsertHandle, out var entity) || entity is not Insert insert || insert.Block.Name != mapping.BlockName)
          throw new IOException($"Не найдена графическая замена объекта #{pair.Key:X}.");
        if (inserts[mapping.BlockName] != 1)
          throw new IOException($"Повторная графическая замена объекта #{pair.Key:X}.");
        if (insert.Block.Entities.Count != mapping.GeneratedCount)
          throw new IOException($"Неполная графическая замена объекта #{pair.Key:X}.");
      }
      else if (!actual.TryGetValue(pair.Key, out var entity) || entity.GetType().Name != pair.Value)
      {
        throw new IOException($"При преобразовании потерян объект #{pair.Key:X} ({pair.Value}).");
      }
    }
    if (entities.Length != source.Count + report.GeneratedCount)
      throw new IOException("Число объектов после преобразования не соответствует отчёту.");
  }

  public static CadDocument VerifyWritten(string path, CadDocument expected, NotificationEventHandler notify)
  {
    var original = Snapshot(expected);
    var restored = DictionaryRecovery.ReadDxf(path, notify);
    var actual = Snapshot(restored);
    var missing = original.Where(pair => !actual.TryGetValue(pair.Key, out var type) || type != pair.Value).ToArray();
    if (missing.Length != 0 || actual.Count != original.Count)
      throw new IOException($"Запись DXF изменила состав объектов: ожидалось {original.Count}, прочитано {actual.Count}; не совпало {missing.Length}. Пример: {string.Join(", ", missing.Take(5).Select(pair => $"#{pair.Key:X} {pair.Value}"))}");
    return restored;
  }
}
