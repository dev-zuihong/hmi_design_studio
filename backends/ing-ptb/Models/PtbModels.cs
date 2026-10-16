using System.Buffers.Binary;

namespace IngPtbTool.Models;

/// <summary>上位机与 PTB 通信使用的命令字。</summary>
public enum PtbCommand : byte
{
    Start = 0xC1,
    ReadResult = 0xC2,
    Configure = 0xC3,
    DeviceId = 0xC4,
    GetVersion = 0xC5,
    GetConfiguration = 0xC6,
    ConfigureDutFirmware = 0xC7,
}

/// <summary>C4 设备 ID 命令的操作类型。</summary>
public enum PtbDeviceIdOperation : byte
{
    Get = 0x00,
    Set = 0x01,
}

/// <summary>PTB 响应中的统一结果码。</summary>
public enum PtbResult : byte
{
    Success = 0x00,
    HeaderError = 0x01,
    UnknownCommand = 0x02,
    CommandLengthError = 0x03,
    CrcError = 0x04,
    TailError = 0x05,
    ParameterError = 0x06,
    PacketLengthError = 0x07,
    FileOpenError = 0x08,
    FileCloseError = 0x09,
    ParameterGetError = 0x0A,
    ParameterSaveError = 0x0B,
    BurnWriteError = 0x10,
    BurnVerifyError = 0x11,
    EraseAllError = 0x12,
    ChipLocked = 0x13,
    UnlockFailed = 0x14,
    LockFailed = 0x15,
    DutSwdTimeout = 0x16,
    DutSerialTimeout = 0x17,
    Busy = 0xE0,
    DutTimeout = 0xE1,
    BurnCountLimit = 0xE2,
}

/// <summary>把协议结果码转换为适合日志和界面显示的文字。</summary>
public static class PtbResultExtensions
{
    public static string ToDescription(this PtbResult result) => result switch
    {
        PtbResult.Success => "成功",
        PtbResult.HeaderError => "帧头错误",
        PtbResult.UnknownCommand => "未知命令",
        PtbResult.CommandLengthError => "命令长度错误",
        PtbResult.CrcError => "CRC 错误",
        PtbResult.TailError => "帧尾错误",
        PtbResult.ParameterError => "命令参数格式或范围错误",
        PtbResult.PacketLengthError => "报文长度错误",
        PtbResult.FileOpenError => "文件打开错误",
        PtbResult.FileCloseError => "文件关闭错误",
        PtbResult.ParameterGetError => "参数获取错误",
        PtbResult.ParameterSaveError => "参数保存错误",
        PtbResult.BurnWriteError => "烧录写入错误",
        PtbResult.BurnVerifyError => "烧录校验错误",
        PtbResult.EraseAllError => "全片擦除错误",
        PtbResult.ChipLocked => "芯片已锁定，无法操作",
        PtbResult.UnlockFailed => "解除锁保护失败",
        PtbResult.LockFailed => "写锁保护失败",
        PtbResult.DutSwdTimeout => "PTB 与 DUT 的 SWD 通信超时",
        PtbResult.DutSerialTimeout => "PTB 与 DUT 的串口通信超时",
        PtbResult.Busy => "操作进行中",
        PtbResult.DutTimeout => "PTB 与 DUT 通信超时",
        PtbResult.BurnCountLimit => "已达到最大烧录次数",
        _ => "未知结果",
    };

    /// <summary>保留枚举名便于定位代码，同时显示协议值和中文含义。</summary>
    public static string ToDisplayText(this PtbResult result) =>
        $"0x{(byte)result:X2}（{result.ToDescription()}，{result}）";
}

/// <summary>RF 测试结果，对应协议中的 <c>rf_test_result_t</c>。</summary>
public sealed record RfTestResult(
    ushort TxCount,
    ushort RxCount,
    sbyte TxPower,
    sbyte RxSensitivity,
    sbyte FrequencyOffset,
    byte Status)
{
    public bool TxCountPassed => (Status & 0x01) != 0;
    public bool RxCountPassed => (Status & 0x02) != 0;
    public bool TxPowerPassed => (Status & 0x04) != 0;
    public bool RxSensitivityPassed => (Status & 0x08) != 0;
    public bool FrequencyOffsetPassed => (Status & 0x10) != 0;
}

/// <summary>产品固件或滚码的烧录结果及失败地址。</summary>
public sealed record BurnResult(byte Result, uint Address)
{
    public bool IsSuccess => Result == 0;
}

/// <summary>C2 返回的完整测试结果，对应协议中的 <c>test_result_t</c>。</summary>
public sealed record PtbTestResult(
    byte FunctionEnable,
    RfTestResult Rf,
    BurnResult Firmware,
    BurnResult RollingCode,
    byte LockResult)
{
    /// <summary>结构在协议中的固定字节数。</summary>
    public const int WireSize = 20;
    public bool RfEnabled => (FunctionEnable & 0x01) != 0;
    public bool FirmwareEnabled => (FunctionEnable & 0x02) != 0;
    public bool RollingCodeEnabled => (FunctionEnable & 0x04) != 0;
    public bool LockEnabled => (FunctionEnable & 0x08) != 0;

    /// <summary>按小端字节序解析 C2 测试结果。</summary>
    public static PtbTestResult Parse(ReadOnlySpan<byte> data)
    {
        if (data.Length < WireSize)
            throw new InvalidDataException("C2 test_result_t 数据长度错误。");
        return new PtbTestResult(
            data[0],
            new RfTestResult(
                BinaryPrimitives.ReadUInt16LittleEndian(data.Slice(1, 2)),
                BinaryPrimitives.ReadUInt16LittleEndian(data.Slice(3, 2)),
                unchecked((sbyte)data[5]), unchecked((sbyte)data[6]), unchecked((sbyte)data[7]), data[8]),
            new BurnResult(data[9], BinaryPrimitives.ReadUInt32LittleEndian(data.Slice(10, 4))),
            new BurnResult(data[14], BinaryPrimitives.ReadUInt32LittleEndian(data.Slice(15, 4))),
            data[19]);
    }
}

/// <summary>单台 PTB 设备在上位机中的业务状态。</summary>
public enum DeviceSessionState
{
    Disconnected,
    Connecting,
    Ready,
    SendingConfig,
    Configured,
    Starting,
    Running,
    Completed,
    Failed,
}

/// <summary>产品固件烧录前的 Flash 擦除方式。</summary>
public enum EraseMode : byte
{
    Demand = 0,
    All = 1,
}

/// <summary>产品固件操作，对应协议中的 <c>fw_func_e</c>。</summary>
public enum FirmwareFunction : byte
{
    None = 0,
    Burn = 1,
    Verify = 2,
}

/// <summary>与目标芯片通信并烧录固件的接口。</summary>
public enum BurnMode : byte
{
    Uart = 0,
    Swd = 1,
}

/// <summary>产品固件烧录完成后的芯片动作。</summary>
public enum PostBurnAction : byte
{
    Halt = 0,
    ResetAndRun = 1,
}

/// <summary>滚码数据的生成与编码模式。</summary>
public enum RollingCodeMode : byte
{
    VariableLengthHex = 0,
}

/// <summary>协议使用的目标芯片类型。</summary>
public enum PtbTargetChip : byte
{
    Ing918 = 0,
    Ing916 = 1,
    Ing920 = 2,
}

/// <summary>目标芯片的显示名称、默认地址和 Flash 范围。</summary>
public static class PtbTargetChipExtensions
{
    public static string ToDisplayName(this PtbTargetChip chip) => chip switch
    {
        PtbTargetChip.Ing916 => "ING916",
        PtbTargetChip.Ing918 => "ING918",
        PtbTargetChip.Ing920 => "ING920",
        _ => $"Unknown ({(byte)chip})",
    };

    public static uint DefaultFirmwareAddress(this PtbTargetChip chip) => chip switch
    {
        PtbTargetChip.Ing918 => 0x00800000,
        _ => 0x02002000,
    };

    public static uint DefaultDutFirmwareAddress(this PtbTargetChip chip) => 0x02002000;

    public static uint DefaultRollingCodeAddress(this PtbTargetChip chip) => chip switch
    {
        PtbTargetChip.Ing918 => 0x0083E000,
        _ => 0x0207E000,
    };

    public static (uint Start, uint Size) FlashRange(this PtbTargetChip chip) => chip switch
    {
        PtbTargetChip.Ing918 => (0x00800000, 0x00040000),
        PtbTargetChip.Ing916 => (0x02000000, 0x00080000),
        PtbTargetChip.Ing920 => (0x02000000, 0x00080000),
        _ => (0, 0),
    };
}

/// <summary>USB 枚举得到的 PTB 设备标识。</summary>
public sealed record PtbDeviceInfo(string DevicePath, string Location);

/// <summary>已加载到内存中的一个固件段。</summary>
public sealed record FirmwareSegment(string SourceName, uint DestinationAddress, byte[] Data, string? SourcePath = null)
{
    public uint Crc32 => Protocol.Crc32Ieee.Compute(Data);
    public ulong EndAddressExclusive => (ulong)DestinationAddress + (ulong)Data.Length;
}

/// <summary>C3 下发的产品测试与烧录配置。</summary>
public sealed class BurnConfiguration
{
    /// <summary>ING920 默认 Flash 起始地址。</summary>
    public const uint Ing920FlashStart = 0x02000000;
    /// <summary>ING920 默认 Flash 容量。</summary>
    public const uint Ing920FlashSize = 0x00080000;
    /// <summary>C3 允许携带的最大产品固件记录数。</summary>
    public const int MaxFirmwareRecords = 5;
    /// <summary>产品固件烧录时的 Flash 擦除方式。</summary>
    public EraseMode EraseMode { get; set; } = EraseMode.Demand;
    /// <summary>目标芯片的烧录接口。</summary>
    public BurnMode BurnMode { get; set; } = BurnMode.Swd;
    /// <summary>产品固件与滚码对应的目标芯片。</summary>
    public PtbTargetChip TargetChip { get; set; } = PtbTargetChip.Ing920;
    /// <summary>PTB 开始任务前是否清除芯片锁；清除会擦除全部 Flash。</summary>
    public bool ClearLock { get; set; }
    /// <summary>任务完成后是否锁定目标芯片。</summary>
    public bool LockEnabled { get; set; }
    /// <summary>目标板供电电压，单位 V。</summary>
    public decimal TvccVoltage { get; set; } = 3.3m;
    /// <summary>允许的最大累计烧录次数，0 表示不限制。</summary>
    public uint BurnCount { get; set; }
    /// <summary>烧录完成后的运行状态。</summary>
    public PostBurnAction PostAction { get; set; } = PostBurnAction.Halt;
    /// <summary>产品固件关闭、烧录或校验。</summary>
    public FirmwareFunction FirmwareFunction { get; set; } = FirmwareFunction.Burn;
    /// <summary>是否在 C3 配置中包含用于烧录或校验的产品固件。</summary>
    public bool FirmwareEnabled => FirmwareFunction != FirmwareFunction.None;
    /// <summary>RF 测试配置。</summary>
    public RfConfiguration Rf { get; set; } = new();
    /// <summary>滚码烧录配置。</summary>
    public RollingCodeConfiguration RollingCode { get; set; } = new();
    /// <summary>按 C3 DATA 顺序写入的产品固件段。</summary>
    public List<FirmwareSegment> Firmware { get; } = [];

    /// <summary>创建可独立修改的配置副本。</summary>
    public BurnConfiguration Clone()
    {
        BurnConfiguration clone = new()
        {
            EraseMode = EraseMode,
            BurnMode = BurnMode,
            TargetChip = TargetChip,
            ClearLock = ClearLock,
            LockEnabled = LockEnabled,
            TvccVoltage = TvccVoltage,
            BurnCount = BurnCount,
            PostAction = PostAction,
            FirmwareFunction = FirmwareFunction,
            Rf = Rf with { },
            RollingCode = RollingCode with { },
        };
        clone.Firmware.AddRange(Firmware);
        return clone;
    }
}

/// <summary>RF 测试参数，对应协议中的 <c>rf_config_t</c>。</summary>
public sealed record RfConfiguration
{
    /// <summary>是否执行 RF 测试。</summary>
    public bool Enabled { get; set; }
    /// <summary>完整 RF 频率，单位 MHz；下发为两字节小端整数。</summary>
    public ushort FrequencyMhz { get; set; } = 2402;
    /// <summary>TX 功率通过阈值，单位 dBm。</summary>
    public sbyte TxPower { get; set; } = -35;
    /// <summary>RX 灵敏度通过阈值，单位 dBm。</summary>
    public sbyte RxPower { get; set; } = -35;
    /// <summary>单次测试的总包数。</summary>
    public ushort PacketCount { get; set; } = 100;
    /// <summary>TX 测试通过所需的最少成功包数。</summary>
    public ushort TxPassCount { get; set; } = 80;
    /// <summary>RX 测试通过所需的最少成功包数。</summary>
    public ushort RxPassCount { get; set; } = 80;
    /// <summary>设备 ID 之间的测试延迟基数，单位毫秒。</summary>
    public ushort DelayMilliseconds { get; set; }
    /// <summary>频偏校准值烧录地址。</summary>
    public uint FrequencyOffsetAddress { get; set; } = 0x0207F000;
}

/// <summary>滚码烧录参数，对应协议中的 <c>roll_config_t</c>。</summary>
public sealed record RollingCodeConfiguration
{
    /// <summary>是否执行滚码烧录。</summary>
    public bool Enabled { get; set; }
    /// <summary>滚码模式。</summary>
    public RollingCodeMode Mode { get; set; } = RollingCodeMode.VariableLengthHex;
    /// <summary>滚码总字节数。</summary>
    public byte ByteLength { get; set; } = 8;
    /// <summary>每次成功烧录后的递增步长。</summary>
    public byte Step { get; set; } = 1;
    /// <summary>参与递增的连续字节位图。</summary>
    public uint Bitmask { get; set; } = 0x0000000F;
    /// <summary>兼容八字节以内旧配置的滚码初始值。</summary>
    public ulong InitialValue { get; set; }
    /// <summary>支持最多 32 字节的十六进制滚码初始值。</summary>
    public string InitialValueHex { get; set; } = string.Empty;
    /// <summary>滚码写入目标地址。</summary>
    public uint Address { get; set; } = 0x0207E000;
}
