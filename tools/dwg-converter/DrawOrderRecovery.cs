using System.Diagnostics.CodeAnalysis;
using System.Reflection;
using ACadSharp;
using ACadSharp.Entities;
using ACadSharp.Objects;
using ACadSharp.Tables;

public sealed record DrawOrderRecoveryReport(int Blocks, int SortEntries, int Replacements);

// Ключ сортировки — число порядка, а не ссылка: при замене меняется только сущность.
public sealed class DrawOrderRecovery
{
  private sealed record Entry(ulong Handle, ulong SortKey);
  private sealed record BlockSnapshot(ulong Handle, ulong[] Sequence, Entry[] ExplicitEntries);
  private readonly CadDocument sourceDocument;
  private readonly BlockSnapshot[] blocks;

  private DrawOrderRecovery(CadDocument document, BlockSnapshot[] snapshots)
  {
    sourceDocument = document;
    blocks = snapshots;
  }

  public static DrawOrderRecovery Capture(CadDocument document)
  {
    var snapshots = new List<BlockSnapshot>();
    foreach (var block in document.BlockRecords)
    {
      var sequence = block.Entities.Select(e => e.Handle).ToArray();
      var entries = ReadEntries(block);
      snapshots.Add(new(block.Handle, sequence, entries));
    }
    return new(document, snapshots.ToArray());
  }

  public DrawOrderRecoveryReport Apply(CadDocument document, ProxyConversionReport conversion)
  {
    if (!ReferenceEquals(sourceDocument, document))
      throw new IOException("Снимок порядка относится к другому документу.");
    var mapping = Mapping(conversion);
    var sourceHandles = blocks.SelectMany(b => b.Sequence).ToHashSet();
    if (mapping.Keys.Any(h => !sourceHandles.Contains(h)))
      throw new IOException("Отчёт замены ссылается на отсутствующую исходную сущность.");
    var staged = new List<(BlockRecord Block, Entity[] Sequence, Entry[] Entries)>();
    foreach (var snapshot in blocks)
    {
      if (!snapshot.Sequence.Any(mapping.ContainsKey)) continue;
      var block = document.GetCadObject<BlockRecord>(snapshot.Handle)
        ?? throw new IOException($"Потерян владелец порядка {snapshot.Handle:X}.");
      var sequence = snapshot.Sequence.Select(h => Resolve(document, block, mapping.GetValueOrDefault(h, h))).ToArray();
      if (block.Entities.Count != sequence.Length || !block.Entities.ToHashSet().SetEquals(sequence))
        throw new IOException($"Изменён состав владельца порядка {snapshot.Handle:X}.");
      var entries = snapshot.ExplicitEntries.Select(e => new Entry(mapping.GetValueOrDefault(e.Handle, e.Handle), e.SortKey)).ToList();
      var explicitHandles = snapshot.ExplicitEntries.Select(e => e.Handle).ToHashSet();
      foreach (var original in snapshot.Sequence)
        if (mapping.TryGetValue(original, out var replacement) && !explicitHandles.Contains(original))
          entries.Add(new(replacement, original));
      staged.Add((block, sequence, entries.ToArray()));
    }
    // Все ссылки проверяются до изменения таблиц. Сами объекты не удаляются и не присоединяются заново.
    foreach (var item in staged)
    {
      RestoreSequence(item.Block, item.Sequence);
      var table = item.Block.CreateSortEntitiesTable();
      table.Clear();
      foreach (var entry in item.Entries)
        table.Add(Resolve(document, item.Block, entry.Handle), entry.SortKey);
    }
    Verify(document, conversion);
    return new(staged.Count, staged.Sum(b => b.Entries.Length), mapping.Count);
  }

  // Можно проверить также документ, повторно прочитанный из записанного DXF.
  public void Verify(CadDocument document, ProxyConversionReport conversion)
  {
    var mapping = Mapping(conversion);
    foreach (var snapshot in blocks)
    {
      var block = document.GetCadObject<BlockRecord>(snapshot.Handle)
        ?? throw new IOException($"Потерян блок порядка {snapshot.Handle:X}.");
      var expectedSequence = snapshot.Sequence.Select(h => mapping.GetValueOrDefault(h, h)).ToArray();
      if (!block.Entities.Select(e => e.Handle).SequenceEqual(expectedSequence))
        throw new IOException($"Изменён порядок записей блока {snapshot.Handle:X}.");
      var originalKeys = snapshot.ExplicitEntries.ToDictionary(e => e.Handle, e => e.SortKey);
      var actualKeys = ReadEntries(block).ToDictionary(e => e.Handle, e => e.SortKey);
      foreach (var original in snapshot.Sequence)
      {
        var current = mapping.GetValueOrDefault(original, original);
        var expected = originalKeys.GetValueOrDefault(original, original);
        if (actualKeys.GetValueOrDefault(current, current) != expected)
          throw new IOException($"Изменён ключ сортировки сущности {original:X} → {current:X}.");
      }
      foreach (var entry in snapshot.ExplicitEntries)
        if (!actualKeys.TryGetValue(mapping.GetValueOrDefault(entry.Handle, entry.Handle), out var key) || key != entry.SortKey)
          throw new IOException($"Потеряна явная запись порядка {entry.Handle:X}.");
    }
  }

  private static Dictionary<ulong, ulong> Mapping(ProxyConversionReport conversion)
  {
    var mapping = conversion.Mappings.ToDictionary(m => m.SourceHandle, m => m.InsertHandle);
    if (mapping.Values.Distinct().Count() != mapping.Count || mapping.Any(p => p.Key == 0 || p.Value == 0))
      throw new IOException("Некорректное соответствие сущностей при восстановлении порядка.");
    return mapping;
  }

  private static Entity Resolve(CadDocument document, BlockRecord owner, ulong handle)
  {
    var entity = document.GetCadObject<Entity>(handle);
    if (entity == null || !ReferenceEquals(entity.Owner, owner))
      throw new IOException($"Сущность порядка {handle:X} не принадлежит блоку {owner.Handle:X}.");
    return entity;
  }

  private static Entry[] ReadEntries(BlockRecord block)
  {
    if (block.SortEntitiesTable is not SortEntitiesTable table) return [];
    if (!ReferenceEquals(table.BlockOwner, block))
      throw new IOException($"Неверный владелец таблицы порядка {block.Handle:X}.");
    var entries = new List<Entry>();
    var seen = new HashSet<ulong>();
    foreach (var sorter in table)
    {
      if (sorter.Entity is not Entity entity || entity.Handle == 0 || !ReferenceEquals(entity.Owner, block)
        || !block.Entities.Contains(entity) || !seen.Add(entity.Handle))
        throw new IOException($"Некорректная ссылка в таблице порядка блока {block.Handle:X}.");
      entries.Add(new(entity.Handle, sorter.SortHandle));
    }
    return entries.ToArray();
  }

  [DynamicDependency("_entries", typeof(CadObjectCollection<Entity>))]
  private static void RestoreSequence(BlockRecord block, Entity[] sequence)
  {
    if (typeof(CadDocument).Assembly.GetName().Version != new Version(3, 8, 35, 0))
      throw new IOException("Восстановление порядка проверено только для ACadSharp 3.8.35.");
    var entries = typeof(CadObjectCollection<Entity>).GetField("_entries", BindingFlags.Instance | BindingFlags.NonPublic)?.GetValue(block.Entities) as HashSet<Entity>
      ?? throw new IOException("Недоступен порядок исходной коллекции сущностей.");
    // Публичные Remove/Add сбрасывают handle и связи. Меняется только последовательность перечисления.
    entries.Clear();
    foreach (var entity in sequence) entries.Add(entity);
    if (!block.Entities.SequenceEqual(sequence) || sequence.Any(e => e.Owner != block || e.Handle == 0))
      throw new IOException("Не удалось восстановить порядок без изменения идентификаторов и владельцев.");
  }
}
