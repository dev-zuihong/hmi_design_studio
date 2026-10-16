using IngPtbTool.Models;
using System.Text.Json.Serialization;

namespace IngPtbTool.Configuration;

/// <summary>读写上位机本地的设备设置配置文件。</summary>
public static class ConfigurationFileService
{
    /// <summary>
    /// 保存产品配置。保留此重载兼容只使用 C3 配置的调用方，新的界面应使用
    /// <see cref="SaveSettingsAsync(string, DeviceSettingsConfiguration, CancellationToken)" />。
    /// </summary>
    public static async Task SaveAsync(string path, BurnConfiguration config, CancellationToken cancellationToken = default)
    {
        await SaveSettingsAsync(path, new DeviceSettingsConfiguration
        {
            ProductConfiguration = config,
        }, cancellationToken);
    }

    /// <summary>保存全部设置和固件源文件路径，不嵌入固件数据。</summary>
    public static async Task SaveSettingsAsync(string path, DeviceSettingsConfiguration settings,
        CancellationToken cancellationToken = default)
    {
        BurnConfigurationDocument document = BurnConfigurationDocument.FromSettings(settings);
        await using FileStream stream = File.Create(path);
        await System.Text.Json.JsonSerializer.SerializeAsync(
            stream, document, ConfigurationJsonContext.Default.BurnConfigurationDocument, cancellationToken);
    }

    /// <summary>加载配置文件中的产品配置，并从记录的源路径重新读取产品固件。</summary>
    public static async Task<BurnConfiguration> LoadAsync(string path, CancellationToken cancellationToken = default)
    {
        DeviceSettingsConfiguration settings = await LoadSettingsAsync(path, cancellationToken);
        return settings.ProductConfiguration;
    }

    /// <summary>加载全部设置，并从记录的源路径重新读取产品和测试固件。</summary>
    public static async Task<DeviceSettingsConfiguration> LoadSettingsAsync(string path,
        CancellationToken cancellationToken = default)
    {
        await using FileStream stream = File.OpenRead(path);
        BurnConfigurationDocument? document = await System.Text.Json.JsonSerializer.DeserializeAsync(
            stream, ConfigurationJsonContext.Default.BurnConfigurationDocument, cancellationToken);
        if (document is null) throw new InvalidDataException("配置文件内容为空。");
        if (document.Version is not (2 or 3 or 4))
            throw new InvalidDataException($"不支持的配置文件版本：{document.Version}。");
        return document.ToSettings(Path.GetDirectoryName(Path.GetFullPath(path))!);
    }
}

/// <summary>运行时使用的完整设备设置，不包含设备 ID。</summary>
public sealed class DeviceSettingsConfiguration
{
    public BurnConfiguration ProductConfiguration { get; init; } = new();
    public PtbTargetChip DutTargetChip { get; init; } = PtbTargetChip.Ing920;
    public uint DutFirmwareAddress { get; init; } = 0x02002000;
    public FirmwareSegment? DutFirmware { get; init; }

    /// <summary>旧版配置文件没有测试固件页，加载时应保留当前测试固件界面。</summary>
    public bool IncludesDutSettings { get; init; } = true;
}

/// <summary>可序列化的完整设备设置文件结构。</summary>
public sealed class BurnConfigurationDocument
{
    public int Version { get; set; } = 4;
    public PtbTargetChip TargetChip { get; set; } = PtbTargetChip.Ing920;
    public BurnMode BurnMode { get; set; } = BurnMode.Swd;
    public EraseMode EraseMode { get; set; }
    public bool ClearLock { get; set; }
    public bool LockEnabled { get; set; }
    public decimal TvccVoltage { get; set; } = 3.3m;
    public uint BurnCount { get; set; }
    public PostBurnAction PostAction { get; set; }
    public FirmwareFunction FirmwareFunction { get; set; } = FirmwareFunction.Burn;
    /// <summary>仅用于读取 V2/V3 配置中的固件开关，新文件保存三态操作。</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public bool? FirmwareEnabled { get; set; }
    public RfConfiguration Rf { get; set; } = new();
    public RollingCodeConfiguration RollingCode { get; set; } = new();
    public List<FirmwareDocument> Firmware { get; set; } = [];
    public PtbTargetChip DutTargetChip { get; set; } = PtbTargetChip.Ing920;
    public uint DutFirmwareAddress { get; set; } = 0x02002000;
    public FirmwareDocument? DutFirmware { get; set; }

    public static BurnConfigurationDocument FromConfiguration(BurnConfiguration config) =>
        FromSettings(new DeviceSettingsConfiguration
        {
            ProductConfiguration = config,
        });

    public static BurnConfigurationDocument FromSettings(DeviceSettingsConfiguration settings)
    {
        BurnConfiguration config = settings.ProductConfiguration;
        return new BurnConfigurationDocument
        {
            TargetChip = config.TargetChip,
            BurnMode = config.BurnMode,
            EraseMode = config.EraseMode,
            ClearLock = config.ClearLock,
            LockEnabled = config.LockEnabled,
            TvccVoltage = config.TvccVoltage,
            BurnCount = config.BurnCount,
            PostAction = config.PostAction,
            FirmwareFunction = config.FirmwareFunction,
            Rf = config.Rf with { },
            RollingCode = config.RollingCode with { },
            Firmware = config.Firmware
                .Select(segment => FirmwareDocument.FromSegment(segment, "产品固件"))
                .ToList(),
            DutTargetChip = settings.DutTargetChip,
            DutFirmwareAddress = settings.DutFirmware?.DestinationAddress ?? settings.DutFirmwareAddress,
            DutFirmware = settings.DutFirmware is null
                ? null
                : FirmwareDocument.FromSegment(settings.DutFirmware, "测试固件"),
        };
    }

    public BurnConfiguration ToConfiguration(string configurationDirectory)
    {
        if (Firmware.Count > BurnConfiguration.MaxFirmwareRecords)
            throw new InvalidDataException("配置文件中的产品固件数量超过 5。 ");

        FirmwareFunction function = Version >= 4
            ? FirmwareFunction
            : FirmwareEnabled != false ? Models.FirmwareFunction.Burn : Models.FirmwareFunction.None;
        if (!Enum.IsDefined(function))
            throw new InvalidDataException($"配置文件中的产品固件操作无效：{function}。");

        BurnConfiguration config = new()
        {
            TargetChip = TargetChip,
            BurnMode = BurnMode,
            EraseMode = EraseMode,
            ClearLock = ClearLock,
            LockEnabled = LockEnabled,
            TvccVoltage = TvccVoltage,
            BurnCount = BurnCount,
            PostAction = PostAction,
            FirmwareFunction = function,
            Rf = Rf with { },
            RollingCode = RollingCode with { },
        };
        foreach (FirmwareDocument item in Firmware)
            config.Firmware.Add(LoadFirmware(item, configurationDirectory, "产品固件"));
        return config;
    }

    public DeviceSettingsConfiguration ToSettings(string configurationDirectory)
    {
        if (!Enum.IsDefined(DutTargetChip))
            throw new InvalidDataException($"配置文件中的测试固件目标芯片无效：{DutTargetChip}。");

        return new DeviceSettingsConfiguration
        {
            ProductConfiguration = ToConfiguration(configurationDirectory),
            DutTargetChip = DutTargetChip,
            DutFirmwareAddress = DutFirmware?.DestinationAddress ?? DutFirmwareAddress,
            DutFirmware = DutFirmware is null
                ? null
                : LoadFirmware(DutFirmware, configurationDirectory, "测试固件"),
            IncludesDutSettings = Version >= 3,
        };
    }

    private static FirmwareSegment LoadFirmware(FirmwareDocument item, string configurationDirectory,
        string firmwareDescription)
    {
        if (string.IsNullOrWhiteSpace(item.FilePath))
            throw new InvalidDataException($"配置文件中的{firmwareDescription}路径为空。");
        string path = Path.GetFullPath(item.FilePath, configurationDirectory);
        if (!File.Exists(path))
            throw new FileNotFoundException($"找不到{firmwareDescription}文件：{path}", path);

        if (Path.GetExtension(path).Equals(".bin", StringComparison.OrdinalIgnoreCase))
            return FirmwareLoader.LoadBin(path, item.DestinationAddress);

        IReadOnlyList<FirmwareSegment> segments = FirmwareLoader.LoadIntelHex(path);
        if (segments.Count != 1)
            throw new InvalidDataException($"{firmwareDescription} {path} 必须只包含一个连续地址段。");
        return segments[0] with { DestinationAddress = item.DestinationAddress };
    }
}

/// <summary>配置文件中保存的产品固件路径和目标地址。</summary>
public sealed class FirmwareDocument
{
    public bool Enabled { get; set; } = true;
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? AssetId { get; set; }
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public uint? SourceAddress { get; set; }
    public string FilePath { get; set; } = string.Empty;
    public uint DestinationAddress { get; set; }

    public static FirmwareDocument FromSegment(FirmwareSegment segment, string firmwareDescription)
    {
        if (string.IsNullOrWhiteSpace(segment.SourcePath))
            throw new InvalidOperationException($"{firmwareDescription} {segment.SourceName} 缺少源文件路径。");
        return new FirmwareDocument
        {
            FilePath = Path.GetFullPath(segment.SourcePath),
            DestinationAddress = segment.DestinationAddress,
        };
    }
}
