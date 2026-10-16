using System.Buffers.Binary;
using IngPtbTool.Models;
using IngPtbTool.Protocol;

namespace IngPtbTool.Configuration;

/// <summary>构建、解析并分帧 C3 产品配置二进制数据。</summary>
public static class DataPackageBuilder
{
    /// <summary>基础（10）、RF（17）、滚码（44）和产品固件（23）配置组成的固定头长度。</summary>
    public const int FixedHeaderSize = 94;
    /// <summary>单个产品固件 <c>bin_info_t</c> 的长度。</summary>
    public const int RecordHeaderSize = 8;
    /// <summary>C3 数据帧除阶段 code 外可携带的数据长度。</summary>
    public const int C3ChunkSize = PtbTransferFrameBuilder.DataChunkSize;
    /// <summary>旧版分帧协议的兼容常量；新协议不再限制帧数。</summary>
    [Obsolete("C3 uses the staged transfer protocol and has no one-byte frame-count limit.")]
    public const int C3MaxFrameCount = int.MaxValue;
    /// <summary>旧版分帧协议的兼容常量；新协议不再限制包长。</summary>
    [Obsolete("C3 uses the staged transfer protocol and has no one-byte frame-count limit.")]
    public const int C3MaxPackageSize = int.MaxValue;

    /// <summary>检查配置字段、固件范围、重叠关系和协议容量。</summary>
    public static IReadOnlyList<string> Validate(BurnConfiguration config, bool allowFirmwareOverlap = false)
    {
        List<string> errors = [];
        if (!Enum.IsDefined(config.TargetChip))
            errors.Add("目标芯片类型无效。");
        if (!Enum.IsDefined(config.BurnMode))
            errors.Add("烧录方式无效。");
        if (!Enum.IsDefined(config.FirmwareFunction))
            errors.Add("产品固件操作无效。");
        if (config.FirmwareEnabled && !Enum.IsDefined(config.EraseMode))
            errors.Add("擦除方式无效。");
        if (!Enum.IsDefined(config.PostAction))
            errors.Add("烧录后动作无效。");

        if (config.TvccVoltage < 1.8m || config.TvccVoltage > 3.3m || config.TvccVoltage * 10 != decimal.Truncate(config.TvccVoltage * 10))
            errors.Add("TVCC 必须为 1.8 V..3.3 V，步长 0.1 V。");

        if (config.Rf.Enabled)
        {
            if (config.Rf.FrequencyMhz < 2402 || config.Rf.FrequencyMhz > 2480 || config.Rf.FrequencyMhz % 2 != 0)
                errors.Add("RF 频点必须为 2402..2480 MHz，步进 2 MHz。");
            if (config.Rf.PacketCount == 0)
                errors.Add("RF 总包数必须为 1..65535。");
            if (config.Rf.TxPassCount > config.Rf.PacketCount)
                errors.Add("RF TX 通过包数不能超过总包数。");
            if (config.Rf.RxPassCount > config.Rf.PacketCount)
                errors.Add("RF RX 通过包数不能超过总包数。");
        }

        if (config.RollingCode.Enabled)
        {
            if (!Enum.IsDefined(config.RollingCode.Mode)) errors.Add("滚码模式无效。");
            if (config.RollingCode.ByteLength is < 4 or > 32)
                errors.Add("滚码字节长度必须为 4..32。");
            if (config.RollingCode.Step == 0) errors.Add("滚码步进必须为 1..255。");
            if (config.RollingCode.Bitmask == 0) errors.Add("滚码位域不能为 0。");
            if (config.RollingCode.ByteLength is > 0 and < 32 &&
                (config.RollingCode.Bitmask >> config.RollingCode.ByteLength) != 0)
                errors.Add("滚码位域不能超过所选字节长度。");
            if (!IsContiguousMask(config.RollingCode.Bitmask))
                errors.Add("滚码位域必须选择连续的 Byte。");
            try { _ = ParseInitialValue(config.RollingCode); }
            catch (Exception ex) { errors.Add(ex.Message); }
            ulong rollingEnd = (ulong)config.RollingCode.Address + config.RollingCode.ByteLength;
            (uint flashStart, uint flashSize) = config.TargetChip.FlashRange();
            ulong flashEnd = (ulong)flashStart + flashSize;
            if (config.RollingCode.Address < flashStart || rollingEnd > flashEnd)
                errors.Add($"滚码烧录地址超出 {config.TargetChip.ToDisplayName()} Flash 范围。");
        }

        if (config.FirmwareEnabled && config.Firmware.Count is < 1 or > BurnConfiguration.MaxFirmwareRecords)
            errors.Add("启用固件烧录或校验时，固件记录数必须为 1..5。");

        FirmwareSegment[] ordered = config.FirmwareEnabled
            ? config.Firmware.OrderBy(x => x.DestinationAddress).ToArray()
            : [];
        for (int i = 0; i < ordered.Length; i++)
        {
            FirmwareSegment current = ordered[i];
            if (current.Data.Length == 0) errors.Add($"{current.SourceName} 为空文件。");
            (uint flashStart, uint flashSize) = config.TargetChip.FlashRange();
            ulong flashEnd = (ulong)flashStart + flashSize;
            if (current.DestinationAddress < flashStart || current.EndAddressExclusive > flashEnd)
                errors.Add($"{current.SourceName} 超出 {config.TargetChip.ToDisplayName()} Flash 范围。");
            if (!allowFirmwareOverlap && i > 0 && ordered[i - 1].EndAddressExclusive > current.DestinationAddress)
                errors.Add($"{current.SourceName} 与其他固件区间重叠。");
        }

        long packageSize = FixedHeaderSize + ordered.Sum(x => RecordHeaderSize + (long)x.Data.Length);
        if (packageSize > int.MaxValue)
            errors.Add("配置包超过上位机可构建的单个字节数组范围。");
        // The staged C3 transfer carries the total file length in its start frame;
        // it is limited only by the byte[]/uint32 representation.
        return errors;
    }

    /// <summary>按协议 packed 布局和小端字节序生成完整 C3 DATA。</summary>
    public static byte[] Build(BurnConfiguration config, bool allowFirmwareOverlap = false)
    {
        IReadOnlyList<string> errors = Validate(config, allowFirmwareOverlap);
        if (errors.Count != 0) throw new InvalidOperationException(string.Join(Environment.NewLine, errors));

        IReadOnlyList<FirmwareSegment> firmware = config.FirmwareEnabled ? config.Firmware : [];
        int length = checked(FixedHeaderSize + firmware.Sum(x => RecordHeaderSize + x.Data.Length));
        byte[] output = new byte[length];
        output[0] = (byte)config.TargetChip;
        output[1] = (byte)config.BurnMode;
        output[2] = config.ClearLock ? (byte)1 : (byte)0;
        output[3] = config.LockEnabled ? (byte)1 : (byte)0;
        output[4] = (byte)config.PostAction;
        output[5] = checked((byte)decimal.ToInt32(config.TvccVoltage * 10));
        BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(6, 4), config.BurnCount);

        if (config.Rf.Enabled)
        {
            output[10] = 1;
            BinaryPrimitives.WriteUInt16LittleEndian(output.AsSpan(11, 2), config.Rf.FrequencyMhz);
            output[13] = unchecked((byte)config.Rf.TxPower);
            output[14] = unchecked((byte)config.Rf.RxPower);
            BinaryPrimitives.WriteUInt16LittleEndian(output.AsSpan(15, 2), config.Rf.PacketCount);
            BinaryPrimitives.WriteUInt16LittleEndian(output.AsSpan(17, 2), config.Rf.TxPassCount);
            BinaryPrimitives.WriteUInt16LittleEndian(output.AsSpan(19, 2), config.Rf.RxPassCount);
            BinaryPrimitives.WriteUInt16LittleEndian(output.AsSpan(21, 2), config.Rf.DelayMilliseconds);
            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(23, 4), config.Rf.FrequencyOffsetAddress);
        }

        if (config.RollingCode.Enabled)
        {
            output[27] = 1;
            output[28] = (byte)config.RollingCode.Mode;
            output[29] = config.RollingCode.ByteLength;
            output[30] = config.RollingCode.Step;
            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(31, 4), config.RollingCode.Bitmask);
            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(35, 4), config.RollingCode.Address);
            ParseInitialValue(config.RollingCode).CopyTo(output, 39);
        }

        output[71] = (byte)config.FirmwareFunction;
        output[72] = config.FirmwareEnabled ? (byte)config.EraseMode : (byte)0;
        output[73] = checked((byte)firmware.Count);

        int offset = FixedHeaderSize;
        for (int i = 0; i < firmware.Count; i++)
        {
            FirmwareSegment item = firmware[i];
            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(74 + i * 4, 4), checked((uint)offset));
            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(offset, 4), item.DestinationAddress);
            BinaryPrimitives.WriteUInt32LittleEndian(output.AsSpan(offset + 4, 4), checked((uint)item.Data.Length));
            item.Data.CopyTo(output, offset + RecordHeaderSize);
            offset += RecordHeaderSize + item.Data.Length;
        }
        return output;
    }

    /// <summary>解析完整 C3 DATA，包括所有产品固件记录。</summary>
    public static BurnConfiguration Parse(ReadOnlySpan<byte> package)
    {
        BurnConfiguration config = ParseSettings(package);
        bool firmwareEnabled = config.FirmwareEnabled;
        int firmwareCount = package[73];
        if (firmwareCount > BurnConfiguration.MaxFirmwareRecords)
            throw new InvalidDataException("Configuration contains more than five firmware records.");
        if (!firmwareEnabled && firmwareCount != 0)
            throw new InvalidDataException("Disabled firmware configuration contains firmware records.");

        int expectedRecordOffset = FixedHeaderSize;
        for (int index = 0; index < firmwareCount; index++)
        {
            uint recordOffsetValue = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(74 + index * 4, 4));
            if (recordOffsetValue != expectedRecordOffset)
                throw new InvalidDataException($"Firmware record {index + 1} has an invalid offset.");
            if (package.Length - expectedRecordOffset < RecordHeaderSize)
                throw new InvalidDataException($"Firmware record {index + 1} header is truncated.");

            uint address = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(expectedRecordOffset, 4));
            uint dataLength = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(expectedRecordOffset + 4, 4));
            if (dataLength == 0 || dataLength > int.MaxValue)
                throw new InvalidDataException($"Firmware record {index + 1} has an invalid length.");

            int recordLength;
            try { recordLength = checked(RecordHeaderSize + (int)dataLength); }
            catch (OverflowException)
            {
                throw new InvalidDataException($"Firmware record {index + 1} length overflows the package.");
            }
            if (recordLength > package.Length - expectedRecordOffset)
                throw new InvalidDataException($"Firmware record {index + 1} data is truncated.");

            byte[] data = package.Slice(expectedRecordOffset + RecordHeaderSize, (int)dataLength).ToArray();
            config.Firmware.Add(new FirmwareSegment($"device-firmware-{index + 1}.bin", address, data));
            expectedRecordOffset = checked(expectedRecordOffset + recordLength);
        }

        if (expectedRecordOffset != package.Length)
            throw new InvalidDataException("Configuration package contains unreferenced trailing data.");
        for (int index = firmwareCount; index < BurnConfiguration.MaxFirmwareRecords; index++)
        {
            if (BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(74 + index * 4, 4)) != 0)
                throw new InvalidDataException("Configuration package contains an unused firmware offset.");
        }

        IReadOnlyList<string> errors = Validate(config, allowFirmwareOverlap: true);
        if (errors.Count != 0)
            throw new InvalidDataException(string.Join(Environment.NewLine, errors));
        return config;
    }

    /// <summary>解析 C3/C6 固定配置头，不读取产品固件内容。</summary>
    public static BurnConfiguration ParseSettings(ReadOnlySpan<byte> package)
    {
        if (package.Length < FixedHeaderSize)
            throw new InvalidDataException("Configuration package is shorter than its fixed header.");
        PtbTargetChip targetChip = (PtbTargetChip)package[0];
        BurnMode burnMode = (BurnMode)package[1];
        FirmwareFunction firmwareFunction = (FirmwareFunction)package[71];
        EraseMode eraseMode = firmwareFunction == FirmwareFunction.None
            ? EraseMode.Demand : (EraseMode)package[72];
        PostBurnAction postAction = (PostBurnAction)package[4];
        if (!Enum.IsDefined(targetChip))
            throw new InvalidDataException($"Unknown target chip value {package[0]}.");
        if (!Enum.IsDefined(burnMode))
            throw new InvalidDataException($"Unknown burn mode value {package[1]}.");
        if (!Enum.IsDefined(firmwareFunction))
            throw new InvalidDataException($"Unknown firmware function value {package[71]}.");
        if (!Enum.IsDefined(eraseMode))
            throw new InvalidDataException($"Unknown erase mode value {package[72]}.");
        if (!Enum.IsDefined(postAction))
            throw new InvalidDataException($"Unknown post-burn action value {package[4]}.");

        bool clearLock = ReadFlag(package[2], "chip lock clear");
        bool lockEnabled = ReadFlag(package[3], "chip lock enable");
        bool rfEnabled = ReadFlag(package[10], "RF enable");
        bool rollingEnabled = ReadFlag(package[27], "rolling-code enable");
        BurnConfiguration config = new()
        {
            TargetChip = targetChip,
            BurnMode = burnMode,
            EraseMode = eraseMode,
            ClearLock = clearLock,
            LockEnabled = lockEnabled,
            TvccVoltage = package[5] / 10m,
            BurnCount = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(6, 4)),
            PostAction = postAction,
            FirmwareFunction = firmwareFunction,
        };

        if (rfEnabled)
        {
            config.Rf = new RfConfiguration
            {
                Enabled = true,
                FrequencyMhz = BinaryPrimitives.ReadUInt16LittleEndian(package.Slice(11, 2)),
                TxPower = unchecked((sbyte)package[13]),
                RxPower = unchecked((sbyte)package[14]),
                PacketCount = BinaryPrimitives.ReadUInt16LittleEndian(package.Slice(15, 2)),
                TxPassCount = BinaryPrimitives.ReadUInt16LittleEndian(package.Slice(17, 2)),
                RxPassCount = BinaryPrimitives.ReadUInt16LittleEndian(package.Slice(19, 2)),
                DelayMilliseconds = BinaryPrimitives.ReadUInt16LittleEndian(package.Slice(21, 2)),
                FrequencyOffsetAddress = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(23, 4)),
            };
        }

        if (rollingEnabled)
        {
            RollingCodeMode mode = (RollingCodeMode)package[28];
            if (!Enum.IsDefined(mode))
                throw new InvalidDataException($"Unknown rolling-code mode value {package[28]}.");
            byte byteLength = package[29];
            uint bitmask = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(31, 4));
            if (byteLength is < 4 or > 32 || package[30] == 0 ||
                (byteLength < 32 && (bitmask >> byteLength) != 0) ||
                !IsContiguousMask(bitmask))
                throw new InvalidDataException("Invalid rolling-code length, step, or byte mask.");
            config.RollingCode = new RollingCodeConfiguration
            {
                Enabled = true,
                Mode = mode,
                ByteLength = byteLength,
                Step = package[30],
                Bitmask = bitmask,
                InitialValue = BinaryPrimitives.ReadUInt64LittleEndian(package.Slice(39, 8)),
                InitialValueHex = FormatInitialValueHex(package.Slice(39, 32), package[29]),
                Address = BinaryPrimitives.ReadUInt32LittleEndian(package.Slice(35, 4)),
            };
        }

        return config;
    }

    private static bool ReadFlag(byte value, string fieldName) => value switch
    {
        0 => false,
        1 => true,
        _ => throw new InvalidDataException($"Invalid {fieldName} value {value}."),
    };

    private static bool IsContiguousMask(uint mask)
    {
        if (mask == 0) return false;
        while ((mask & 1u) == 0u) mask >>= 1;
        return (mask & (mask + 1u)) == 0u;
    }

    /// <summary>把界面中的大端十六进制文本转换为协议中的 32 字节小端值。</summary>
    private static byte[] ParseInitialValue(RollingCodeConfiguration rolling)
    {
        string text = string.IsNullOrWhiteSpace(rolling.InitialValueHex)
            ? rolling.InitialValue.ToString("X")
            : rolling.InitialValueHex.Trim().TrimStart("0x".ToCharArray());
        if (text.Length == 0) text = "0";
        if (text.Length > rolling.ByteLength * 2 || text.Length > 64 || !text.All(Uri.IsHexDigit))
            throw new InvalidOperationException("滚码开始值必须是不超过长度限制的十六进制数值。");
        if ((text.Length & 1) != 0) text = "0" + text;
        byte[] result = new byte[32];
        byte[] parsed = Convert.FromHexString(text);
        Array.Reverse(parsed);
        parsed.CopyTo(result, 0);
        return result;
    }

    private static string FormatInitialValueHex(ReadOnlySpan<byte> littleEndianValue, int length)
    {
        length = Math.Clamp(length, 1, 32);
        // Values up to eight bytes are represented by InitialValue for compatibility
        // with existing configuration files; larger values require the HEX field.
        if (length <= sizeof(ulong)) return string.Empty;
        byte[] value = littleEndianValue[..length].ToArray();
        Array.Reverse(value);
        return Convert.ToHexString(value);
    }

    /// <summary>生成需要用户确认的产品固件地址重叠信息。</summary>
    public static IReadOnlyList<string> GetFirmwareOverlapMessages(BurnConfiguration config)
    {
        if (!config.FirmwareEnabled) return [];

        List<string> messages = [];
        for (int i = 0; i < config.Firmware.Count; i++)
        {
            FirmwareSegment first = config.Firmware[i];
            for (int j = i + 1; j < config.Firmware.Count; j++)
            {
                FirmwareSegment second = config.Firmware[j];
                ulong start = Math.Max(first.DestinationAddress, second.DestinationAddress);
                ulong end = Math.Min(first.EndAddressExclusive, second.EndAddressExclusive);
                if (start >= end) continue;

                messages.Add($"{first.SourceName} ↔ {second.SourceName}：0x{start:X8}–0x{end - 1:X8}");
            }
        }
        return messages;
    }

    /// <summary>按 C3 三阶段协议生成开始、数据和结束请求帧。</summary>
    public static IReadOnlyList<PtbFrame> CreateC3Frames(byte[] package)
    {
        ArgumentNullException.ThrowIfNull(package);
        return PtbTransferFrameBuilder.CreateFrames(PtbCommand.Configure, package);
    }

}
