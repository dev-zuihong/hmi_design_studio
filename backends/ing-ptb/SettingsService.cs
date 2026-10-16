using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text;
using IngPtbTool.Configuration;
using IngPtbTool.Models;

namespace IngPtb.Backend;

public sealed class SettingsService(ResourceRegistry resources)
{
    public static JsonObject Defaults() => ToJson(new BurnConfigurationDocument { FirmwareFunction = FirmwareFunction.None, Rf = new() { Enabled = true } });
    public static BurnConfigurationDocument Parse(JsonNode settings)
    {
        var document = JsonSerializer.Deserialize(settings, ConfigurationJsonContext.Default.BurnConfigurationDocument)
            ?? throw new InvalidDataException("配置为空。");
        if (document.Version is not (2 or 3 or 4)) throw new InvalidDataException("只支持.ptbcfg版本2/3/4。");
        if (document.Version < 4) document.FirmwareFunction = document.FirmwareEnabled == false ? FirmwareFunction.None : FirmwareFunction.Burn;
        if (document.Rf is null || document.RollingCode is null || document.Firmware is null || document.Firmware.Any(f => f is null))
            throw new InvalidDataException("rf、rollingCode和firmware不能为空。");
        if (document.Firmware.Count > BurnConfiguration.MaxFirmwareRecords) throw new InvalidDataException("最多支持5段产品固件。");
        if (!Enum.IsDefined(document.DutTargetChip)) throw new InvalidDataException("测试固件芯片无效。");
        return document;
    }
    public static JsonObject ToJson(BurnConfigurationDocument document) => JsonSerializer.SerializeToNode(document, ConfigurationJsonContext.Default.BurnConfigurationDocument)!.AsObject();
    public JsonObject Import(JsonNode input, JsonObject? references = null)
    {
        BurnConfigurationDocument document = Parse(input);
        var missing = new JsonArray();
        foreach (var firmware in document.Firmware.Where(f => f.Enabled).Concat(document.DutFirmware is null ? [] : new[] { document.DutFirmware }))
        {
            string? id = firmware.AssetId;
            if (id is null && references?[firmware.FilePath] is JsonNode supplied) id = resources.ResolveId(supplied);
            id ??= resources.Find(firmware.FilePath);
            firmware.AssetId = id;
            if (id is null) missing.Add((JsonNode)new JsonObject { ["filePath"] = firmware.FilePath, ["name"] = Path.GetFileName(firmware.FilePath) });
        }
        bool includesDut = document.Version >= 3;
        document.Version = 4; document.FirmwareEnabled = null;
        return new() { ["valid"] = missing.Count == 0, ["settings"] = ToJson(document), ["missingResources"] = missing, ["includesDutSettings"] = includesDut };
    }
    public BurnConfiguration Materialize(JsonNode settings, bool allowOverlap = false)
    {
        var doc = Parse(settings);
        var config = new BurnConfiguration
        {
            TargetChip = doc.TargetChip, BurnMode = doc.BurnMode, EraseMode = doc.EraseMode,
            ClearLock = doc.ClearLock, LockEnabled = doc.LockEnabled, TvccVoltage = doc.TvccVoltage,
            BurnCount = doc.BurnCount, PostAction = doc.PostAction, FirmwareFunction = doc.FirmwareFunction,
            Rf = doc.Rf with { }, RollingCode = doc.RollingCode with { },
        };
        if (config.FirmwareEnabled) foreach (var item in doc.Firmware.Where(f => f.Enabled)) config.Firmware.Add(Load(item));
        var errors = DataPackageBuilder.Validate(config, allowOverlap);
        if (errors.Count != 0) throw new InvalidDataException(string.Join("\n", errors));
        return config;
    }
    public FirmwareSegment Load(FirmwareDocument item)
    {
        string id = item.AssetId ?? resources.Find(item.FilePath) ?? throw new InvalidDataException($"固件尚未导入工程：{item.FilePath}");
        var resource = resources.Get(id);
        byte[] verified = resource.Read();
        if (Path.GetExtension(resource.Name).Equals(".bin", StringComparison.OrdinalIgnoreCase))
            return new(resource.Name, item.DestinationAddress, verified, resource.Path);
        if (Path.GetExtension(resource.Name).ToLowerInvariant() is not (".hex" or ".ihx")) throw new InvalidDataException("固件必须为BIN/HEX/IHX。");
        var segments = FirmwareLoader.ParseIntelHex(Encoding.UTF8.GetString(verified), resource.Name, resource.Path);
        var segment = item.SourceAddress.HasValue ? segments.SingleOrDefault(segment => segment.DestinationAddress == item.SourceAddress.Value)
            : segments.Count == 1 ? segments[0] : segments.SingleOrDefault(segment => segment.DestinationAddress == item.DestinationAddress);
        if (segment is null) throw new InvalidDataException("请选择HEX/IHX中的地址段sourceAddress。");
        return segment with { DestinationAddress = item.DestinationAddress };
    }
    public JsonObject Inspect(string id)
    {
        var resource = resources.Get(id); byte[] bytes = resource.Read();
        var rows = new JsonArray();
        IReadOnlyList<FirmwareSegment> segments;
        string extension = Path.GetExtension(resource.Name).ToLowerInvariant();
        if (extension == ".bin") segments = [new(resource.Name, 0x02002000, bytes, resource.Path)];
        else if (extension is ".hex" or ".ihx") segments = FirmwareLoader.ParseIntelHex(Encoding.UTF8.GetString(bytes), resource.Name, resource.Path);
        else throw new InvalidDataException("固件必须为BIN/HEX/IHX。");
        foreach (var item in segments) rows.Add((JsonNode)new JsonObject { ["assetId"] = id, ["name"] = resource.Name, ["destinationAddress"] = item.DestinationAddress, ["size"] = item.Data.Length, ["crc32"] = $"{item.Crc32:X8}", ["sourceAddress"] = item.DestinationAddress });
        return new() { ["resource"] = resource.Reference(), ["segments"] = rows };
    }
    public JsonObject Export(JsonNode settings)
    {
        var doc = Parse(settings); doc.Version = 4; doc.FirmwareEnabled = null;
        doc.Firmware = doc.Firmware.Where(f => f.Enabled).ToList();
        if (doc.DutFirmware is not null) doc.DutFirmware.DestinationAddress = doc.DutFirmwareAddress;
        var files = new JsonArray();
        foreach (var item in doc.Firmware.Concat(doc.DutFirmware is null ? [] : new[] { doc.DutFirmware }))
        {
            var file = resources.Get(item.AssetId ?? resources.Find(item.FilePath) ?? throw new InvalidDataException($"导出前请补齐固件资源：{item.FilePath}"));
            if (Path.GetExtension(file.Name).ToLowerInvariant() is ".hex" or ".ihx"
                && FirmwareLoader.ParseIntelHex(Encoding.UTF8.GetString(file.Read()), file.Name).Count > 1)
                throw new InvalidDataException("原GUI的.ptbcfg只支持单连续段HEX/IHX；请将选定段转换为BIN后导出兼容配置。");
            string relative = $"firmware/{file.Id}{Path.GetExtension(file.Name).ToLowerInvariant()}";
            item.FilePath = relative;
            var reference = file.Reference(); reference["relativePath"] = relative;
            if (!files.Any(existing => existing?["id"]?.GetValue<string>() == file.Id)) files.Add((JsonNode)reference);
            item.AssetId = null;
        }
        var json = ToJson(doc);
        return new() { ["name"] = "settings.ptbcfg", ["content"] = json.ToJsonString(), ["document"] = json, ["resources"] = files };
    }
    public static JsonObject RollingPreview(JsonNode settings)
    {
        var document = Parse(settings);
        var rolling = document.RollingCode;
        var config = new BurnConfiguration { TargetChip = document.TargetChip, FirmwareFunction = FirmwareFunction.None, RollingCode = rolling with { Enabled = true } };
        var errors = DataPackageBuilder.Validate(config);
        if (errors.Count != 0) throw new InvalidDataException(string.Join("\n", errors));
        byte[] value = DataPackageBuilder.Build(config).AsSpan(39, rolling.ByteLength).ToArray();
        int first = 0; while ((rolling.Bitmask & (1u << first)) == 0) first++;
        int last = first; while (last + 1 < rolling.ByteLength && (rolling.Bitmask & (1u << (last + 1))) != 0) last++;
        var rows = new JsonArray();
        for (int index = 1; index <= 52; index++)
        {
            byte[] display = value.Reverse().ToArray();
            rows.Add((JsonNode)new JsonObject { ["index"] = index, ["hex"] = Convert.ToHexString(display), ["address"] = rolling.Address, ["byteLength"] = rolling.ByteLength, ["step"] = rolling.Step, ["firstByte"] = first + 1, ["lastByte"] = last + 1 });
            for (int step = 0; step < rolling.Step; step++) { int cursor = first; while (cursor <= last && ++value[cursor] == 0) cursor++; }
        }
        return new() { ["rows"] = rows, ["count"] = 52 };
    }

    public JsonObject Describe(JsonObject settings)
    {
        var doc = Parse(settings); var rows = new JsonArray(); var memory = new JsonArray(); var dut = new JsonArray(); var dutMemory = new JsonArray();
        JsonObject DescribeFirmware(FirmwareDocument? item, string id, JsonArray map)
        {
            var row = new JsonObject { ["id"] = id, ["enabled"] = item?.Enabled ?? false, ["file"] = null,
                ["destinationAddress"] = item?.DestinationAddress ?? doc.TargetChip.DefaultFirmwareAddress(), ["size"] = 0, ["crc32"] = "", ["sourceAddress"] = item?.SourceAddress };
            if (item is null || item.AssetId is null && string.IsNullOrEmpty(item.FilePath)) return row;
            try
            {
                var file = resources.Get(item.AssetId ?? resources.Find(item.FilePath) ?? throw new InvalidDataException("资源尚未导入。"));
                var segment = Load(item); row["file"] = file.Reference(); row["size"] = segment.Data.Length; row["crc32"] = $"{segment.Crc32:X8}";
                if (item.Enabled) map.Add((JsonNode)new JsonObject { ["id"] = id, ["label"] = file.Name, ["start"] = item.DestinationAddress, ["size"] = segment.Data.Length });
            }
            catch (Exception error) { row["error"] = error.Message.Length > 128 ? error.Message[..128] : error.Message; }
            return row;
        }
        for (int index = 0; index < 5; index++) rows.Add((JsonNode)DescribeFirmware(index < doc.Firmware.Count ? doc.Firmware[index] : null, index.ToString(), memory));
        var dutItem = doc.DutFirmware;
        if (dutItem is not null) dutItem.DestinationAddress = doc.DutFirmwareAddress;
        dut.Add((JsonNode)DescribeFirmware(dutItem, "0", dutMemory));
        if (doc.RollingCode.Enabled) memory.Add((JsonNode)new JsonObject { ["id"] = "rolling", ["label"] = "滚码", ["start"] = doc.RollingCode.Address, ["size"] = doc.RollingCode.ByteLength });
        if (doc.Rf.Enabled) memory.Add((JsonNode)new JsonObject { ["id"] = "rf", ["label"] = "RF频偏", ["start"] = doc.Rf.FrequencyOffsetAddress, ["size"] = 1 });
        int first = 0, count = 0;
        uint mask = doc.RollingCode.Bitmask;
        while (first < 32 && (mask & (1u << first)) == 0) first++;
        while (first + count < 32 && (mask & (1u << (first + count))) != 0) count++;
        JsonObject preview;
        try { preview = RollingPreview(settings); } catch (Exception error) { preview = new() { ["rows"] = new JsonArray(), ["count"] = 0, ["error"] = error.Message }; }
        var range = doc.TargetChip.FlashRange();
        return new() { ["firmwareRows"] = rows, ["memorySegments"] = memory, ["dutFirmwareRows"] = dut, ["dutSegments"] = dutMemory,
            ["rollingRange"] = new JsonObject { ["startByte"] = first == 32 ? 0 : first, ["byteCount"] = count }, ["rollingPreview"] = preview,
            ["memoryRange"] = new JsonObject { ["start"] = range.Start, ["size"] = range.Size } };
    }

    public void PatchSpecial(JsonObject settings, string path, JsonNode? value)
    {
        if (path == "rollingCode.range")
        {
            int start = value?["startByte"]?.GetValue<int>() ?? -1, count = value?["byteCount"]?.GetValue<int>() ?? -1;
            int length = settings["rollingCode"]?["byteLength"]?.GetValue<int>() ?? 0;
            if (start < 0 || count < 1 || start + count > length || length > 32) throw new InvalidDataException("滚码区间必须在总字节长度内连续选择。");
            settings["rollingCode"]!["bitmask"] = count == 32 ? uint.MaxValue : ((1u << count) - 1) << start;
            return;
        }
        bool isDut = path == "dutFirmware.resource";
        var parts = path.Split('.');
        if (!isDut && (parts.Length != 3 || parts[0] != "firmware" || !int.TryParse(parts[1], out int _) || parts[2] != "resource")) throw new InvalidDataException("未知资源路径。");
        int slot = isDut ? 0 : int.Parse(parts[1]);
        if (slot is < 0 or > 4) throw new InvalidDataException("产品固件槽位必须为0至4。");
        if (value is null)
        {
            if (isDut) settings["dutFirmware"] = null;
            else if (settings["firmware"] is JsonArray old && slot < old.Count) old[slot] = new JsonObject { ["enabled"] = false, ["destinationAddress"] = Parse(settings).TargetChip.DefaultFirmwareAddress(), ["filePath"] = "" };
            return;
        }
        string id = resources.ResolveId(value); var inspected = Inspect(id); var segments = inspected["segments"]!.AsArray();
        if (isDut && segments.Count != 1) throw new InvalidDataException("测试固件必须只有一个连续地址段。");
        if (slot + segments.Count > 5) throw new InvalidDataException("HEX/IHX段数超出剩余固件槽位。");
        var list = settings["firmware"]!.AsArray();
        for (int index = 0; index < segments.Count; index++)
        {
            var source = segments[index]!;
            uint address = source["destinationAddress"]!.GetValue<uint>();
            if (Path.GetExtension(resources.Get(id).Name).Equals(".bin", StringComparison.OrdinalIgnoreCase)) address = isDut ? settings["dutFirmwareAddress"]!.GetValue<uint>() : Parse(settings).TargetChip.DefaultFirmwareAddress();
            var item = new JsonObject { ["assetId"] = id, ["enabled"] = true, ["filePath"] = resources.Get(id).Name, ["sourceAddress"] = source["sourceAddress"]!.DeepClone(), ["destinationAddress"] = address };
            if (isDut) { settings["dutFirmware"] = item; settings["dutFirmwareAddress"] = address; }
            else
            {
                while (list.Count <= slot + index) list.Add((JsonNode)new JsonObject { ["enabled"] = false, ["destinationAddress"] = Parse(settings).TargetChip.DefaultFirmwareAddress(), ["filePath"] = "" });
                list[slot + index] = item;
            }
        }
    }
}
