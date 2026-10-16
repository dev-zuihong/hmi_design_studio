using System.Buffers.Binary;
using IngPtbTool.Models;

namespace IngPtbTool.Protocol;

/// <summary>
/// C3/C7 传输阶段。阶段字节位于请求 PAYLOAD 的第一个字节。
/// </summary>
public enum PtbTransferCode : byte
{
    Start = 0x00,
    Data = 0x01,
    End = 0x02,
}

/// <summary>
/// 按 PTB C3/C7 三阶段协议生成 stop-and-wait 请求帧。
/// </summary>
public static class PtbTransferFrameBuilder
{
    /// <summary>
    /// 单个数据帧可携带的文件数据字节数。PAYLOAD 上限为 254，需扣除 code 字节。
    /// </summary>
    public const int DataChunkSize = PtbFrame.MaxPayloadLength - 1;

    /// <summary>
    /// 生成开始、数据和结束帧。开始帧携带文件长度（uint32 little-endian）。
    /// </summary>
    public static IReadOnlyList<PtbFrame> CreateFrames(
        PtbCommand command, ReadOnlySpan<byte> package)
    {
        byte[] startData = new byte[sizeof(uint)];
        BinaryPrimitives.WriteUInt32LittleEndian(startData, checked((uint)package.Length));
        return CreateFrames(command, startData, package);
    }

    /// <summary>
    /// 生成分阶段帧，并允许 C7 在开始帧附加芯片类型、同时从数据帧中省略该字段。
    /// </summary>
    public static IReadOnlyList<PtbFrame> CreateFrames(
        PtbCommand command, ReadOnlySpan<byte> startData, ReadOnlySpan<byte> transferData)
    {
        if (command is not (PtbCommand.Configure or PtbCommand.ConfigureDutFirmware))
            throw new ArgumentOutOfRangeException(nameof(command), command,
                "Only C3 and C7 commands support staged transfers.");

        byte[] startPayload = new byte[startData.Length + 1];
        startPayload[0] = (byte)PtbTransferCode.Start;
        startData.CopyTo(startPayload.AsSpan(1));

        int dataFrameCount = transferData.Length == 0
            ? 0
            : checked((transferData.Length - 1) / DataChunkSize + 1);
        List<PtbFrame> frames = new(checked(dataFrameCount + 2))
        {
            new(PtbFrame.HostType, command, startPayload),
        };

        for (int offset = 0; offset < transferData.Length;)
        {
            int count = Math.Min(DataChunkSize, transferData.Length - offset);
            byte[] payload = new byte[count + 1];
            payload[0] = (byte)PtbTransferCode.Data;
            transferData.Slice(offset, count).CopyTo(payload.AsSpan(1));
            frames.Add(new PtbFrame(PtbFrame.HostType, command, payload));
            offset += count;
        }

        frames.Add(new PtbFrame(PtbFrame.HostType, command,
            [(byte)PtbTransferCode.End]));
        return frames;
    }
}
