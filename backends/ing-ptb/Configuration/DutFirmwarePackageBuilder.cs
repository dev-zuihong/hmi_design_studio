using System.Buffers.Binary;
using IngPtbTool.Models;
using IngPtbTool.Protocol;

namespace IngPtbTool.Configuration;

/// <summary>构建并分帧 C7 测试固件数据。</summary>
public static class DutFirmwarePackageBuilder
{
    /// <summary>芯片类型、目标地址和固件长度组成的 C7 DATA 头长度。</summary>
    public const int HeaderSize = 9;
    /// <summary>C7 数据帧除阶段 code 外可携带的数据长度。</summary>
    public const int ChunkSize = PtbTransferFrameBuilder.DataChunkSize;
    [Obsolete("C7 uses the staged transfer protocol and has no one-byte frame-count limit.")]
    public const int MaxFrameCount = int.MaxValue;
    [Obsolete("C7 uses the staged transfer protocol and has no one-byte frame-count limit.")]
    public const int MaxPackageSize = int.MaxValue;

    /// <summary>生成 <c>chip_type + bin_info_t + firmware</c> 完整数据。</summary>
    public static byte[] Build(PtbTargetChip chip, FirmwareSegment firmware)
    {
        if (!Enum.IsDefined(chip)) throw new ArgumentOutOfRangeException(nameof(chip));
        ArgumentNullException.ThrowIfNull(firmware);
        if (firmware.Data.Length == 0) throw new InvalidOperationException("测试固件不能为空。");
        int packageLength = checked(HeaderSize + firmware.Data.Length);
        byte[] package = new byte[packageLength];
        package[0] = (byte)chip;
        BinaryPrimitives.WriteUInt32LittleEndian(package.AsSpan(1, 4), firmware.DestinationAddress);
        BinaryPrimitives.WriteUInt32LittleEndian(package.AsSpan(5, 4), checked((uint)firmware.Data.Length));
        firmware.Data.CopyTo(package, HeaderSize);
        return package;
    }

    /// <summary>按 C7 三阶段协议生成开始、数据和结束请求帧。</summary>
    public static IReadOnlyList<PtbFrame> CreateFrames(PtbTargetChip chip, FirmwareSegment firmware)
    {
        byte[] package = Build(chip, firmware);
        // C7 start DATA is file size + chip type; subsequent DATA starts with
        // bin_info_t, so the chip byte in the local package is not transmitted.
        byte[] startData = new byte[sizeof(uint) + sizeof(byte)];
        BinaryPrimitives.WriteUInt32LittleEndian(startData, checked((uint)package.Length - 1));
        startData[sizeof(uint)] = package[0];
        return PtbTransferFrameBuilder.CreateFrames(PtbCommand.ConfigureDutFirmware,
            startData, package.AsSpan(1));
    }
}
