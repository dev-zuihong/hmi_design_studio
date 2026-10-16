using IngPtbTool.Models;

namespace IngPtbTool.Protocol;

/// <summary>一帧已解析的 PTB 协议消息。</summary>
public sealed record PtbFrame(byte Type, PtbCommand Command, byte[] Payload)
{
    public const byte HostType = 0x04;
    public const byte DeviceType = 0xF4;
    public const int MaxPayloadLength = 254;
}

/// <summary>PTB 协议帧的编码与完整帧校验。</summary>
public static class PtbFrameCodec
{
    /// <summary>添加帧头、类型、长度、CRC16 和帧尾。</summary>
    public static byte[] Encode(PtbFrame frame)
    {
        if (frame.Payload.Length > PtbFrame.MaxPayloadLength)
            throw new ArgumentOutOfRangeException(nameof(frame), "Payload exceeds 254 bytes.");

        byte[] output = new byte[frame.Payload.Length + 9];
        output[0] = 0x45;
        output[1] = 0x4A;
        output[2] = frame.Type;
        output[3] = checked((byte)(frame.Payload.Length + 1));
        output[4] = (byte)frame.Command;
        frame.Payload.CopyTo(output, 5);
        ushort crc = Crc16Ptb.Compute(output.AsSpan(0, 5 + frame.Payload.Length));
        // CRC is serialized as a little-endian uint16: low byte first.
        output[^4] = (byte)crc;
        output[^3] = (byte)(crc >> 8);
        output[^2] = 0x4E;
        output[^1] = 0x44;
        return output;
    }

    /// <summary>校验并解析一条边界完整的协议帧。</summary>
    public static bool TryDecode(ReadOnlySpan<byte> data, out PtbFrame? frame, out string? error)
    {
        frame = null;
        error = null;
        if (data.Length < 9) { error = "Frame is shorter than 9 bytes."; return false; }
        int expected = data[3] + 8;
        if (expected != data.Length) { error = "LEN does not match frame length."; return false; }
        if (data[0] != 0x45 || data[1] != 0x4A) { error = "Invalid header."; return false; }
        if (data[^2] != 0x4E || data[^1] != 0x44) { error = "Invalid tail."; return false; }
        // CRC is serialized as a little-endian uint16: low byte first.
        ushort actual = (ushort)(data[^4] | (data[^3] << 8));
        ushort expectedCrc = Crc16Ptb.Compute(data[..^4]);
        if (actual != expectedCrc) { error = "CRC mismatch."; return false; }

        frame = new PtbFrame(data[2], (PtbCommand)data[4], data[5..^4].ToArray());
        return true;
    }
}

/// <summary>从可能拆包、粘包或带噪声的 USB 字节流中恢复协议帧。</summary>
public sealed class PtbFrameStreamParser
{
    private readonly List<byte> buffer = [];

    /// <summary>追加新字节并返回本次恢复出的全部完整帧。</summary>
    public IReadOnlyList<PtbFrame> Append(ReadOnlySpan<byte> bytes)
    {
        buffer.AddRange(bytes.ToArray());
        List<PtbFrame> frames = [];
        while (true)
        {
            int header = FindHeader();
            if (header < 0)
            {
                if (buffer.Count > 0 && buffer[^1] == 0x45) buffer.RemoveRange(0, buffer.Count - 1);
                else buffer.Clear();
                break;
            }
            if (header > 0) buffer.RemoveRange(0, header);
            if (buffer.Count < 4) break;
            int length = buffer[3] + 8;
            if (buffer.Count < length) break;
            byte[] candidate = buffer.GetRange(0, length).ToArray();
            if (PtbFrameCodec.TryDecode(candidate, out PtbFrame? frame, out _))
            {
                frames.Add(frame!);
                buffer.RemoveRange(0, length);
            }
            else
            {
                buffer.RemoveAt(0);
            }
        }
        return frames;
    }

    private int FindHeader()
    {
        for (int i = 0; i + 1 < buffer.Count; i++)
            if (buffer[i] == 0x45 && buffer[i + 1] == 0x4A) return i;
        return -1;
    }
}
