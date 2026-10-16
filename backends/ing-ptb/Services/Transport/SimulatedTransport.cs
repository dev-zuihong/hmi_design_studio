using System.Buffers.Binary;
using System.Threading.Channels;
using IngPtbTool.Configuration;
using IngPtbTool.Models;
using IngPtbTool.Protocol;

namespace IngPtbTool.Services.Transport;

/// <summary>Exercises the same wire codec and stop-and-wait session as USB.</summary>
public sealed class SimulatedTransport(PtbDeviceInfo device, byte id) : IPtbTransport
{
    private readonly Channel<byte[]> replies = Channel.CreateUnbounded<byte[]>();
    private byte[] remaining = [];
    private byte deviceId = id;
    private byte[] header = DataPackageBuilder.Build(new BurnConfiguration { FirmwareFunction = FirmwareFunction.None, Rf = new() { Enabled = true } });
    private MemoryStream? transfer;
    private PtbCommand transferCommand;
    private uint expectedSize;
    private int polls;
    private bool started;
    private bool disposed;
    public int WriteCount { get; private set; }
    public bool DropNextReply { get; set; }
    public int ResponseDelayMs { get; set; } = 2;
    public PtbDeviceInfo Device { get; } = device;
    public bool IsOpen { get; private set; }

    public Task OpenAsync(CancellationToken token) { token.ThrowIfCancellationRequested(); ObjectDisposedException.ThrowIf(disposed, this); IsOpen = true; return Task.CompletedTask; }
    public async ValueTask<int> ReadAsync(Memory<byte> target, CancellationToken token)
    {
        if (remaining.Length == 0) remaining = await replies.Reader.ReadAsync(token);
        int length = Math.Min(target.Length, remaining.Length);
        remaining.AsMemory(0, length).CopyTo(target);
        remaining = remaining[length..];
        return length;
    }
    public async ValueTask WriteAsync(ReadOnlyMemory<byte> data, CancellationToken token)
    {
        if (!IsOpen) throw new IOException("模拟设备已断开。");
        if (!PtbFrameCodec.TryDecode(data.Span, out var frame, out var error)) throw new InvalidDataException(error);
        WriteCount++;
        byte[] result = Process(frame!);
        await Task.Delay(ResponseDelayMs, token);
        if (DropNextReply) { DropNextReply = false; return; }
        byte[] response = PtbFrameCodec.Encode(new PtbFrame(PtbFrame.DeviceType, frame!.Command, result));
        // Split every reply to exercise incremental parsing.
        replies.Writer.TryWrite(response[..3]); replies.Writer.TryWrite(response[3..]);
    }
    private byte[] Process(PtbFrame frame)
    {
        byte[] data = frame.Payload;
        switch (frame.Command)
        {
            case PtbCommand.GetVersion: return [0, 1, 0, 3, 1];
            case PtbCommand.DeviceId:
                if (data.Length == 1 && data[0] == 0) return [0, deviceId];
                if (data.Length == 2 && data[0] == 1) { deviceId = data[1]; return [0]; }
                return [(byte)PtbResult.ParameterError];
            case PtbCommand.GetConfiguration: return [0, .. header];
            case PtbCommand.Configure:
            case PtbCommand.ConfigureDutFirmware:
                if (data.Length == 0) return [(byte)PtbResult.CommandLengthError];
                if (data[0] == 0)
                {
                    if (transfer is not null) return [(byte)PtbResult.Busy];
                    if (data.Length != (frame.Command == PtbCommand.Configure ? 5 : 6)) return [(byte)PtbResult.CommandLengthError];
                    expectedSize = BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(1, 4));
                    if (expectedSize > 8 * 1024 * 1024) return [(byte)PtbResult.ParameterError];
                    transfer = new MemoryStream(); transferCommand = frame.Command; return [0];
                }
                if (transfer is null || transferCommand != frame.Command) return [(byte)PtbResult.ParameterError];
                if (data[0] == 1)
                {
                    if (data.Length < 2 || transfer.Length + data.Length - 1 > expectedSize) { transfer.Dispose(); transfer = null; return [(byte)PtbResult.ParameterError]; }
                    transfer.Write(data.AsSpan(1)); return [0];
                }
                if (data[0] == 2 && data.Length == 1)
                {
                    byte[] package = transfer.ToArray(); transfer.Dispose(); transfer = null;
                    if (package.Length != expectedSize) return [(byte)PtbResult.ParameterError];
                    if (frame.Command == PtbCommand.Configure)
                    {
                        try { DataPackageBuilder.Parse(package); header = package[..DataPackageBuilder.FixedHeaderSize]; }
                        catch { return [(byte)PtbResult.ParameterError]; }
                    }
                    return [0];
                }
                return [(byte)PtbResult.ParameterError];
            case PtbCommand.Start:
                if (started) return [(byte)PtbResult.Busy];
                started = true; polls = 0; return [0];
            case PtbCommand.ReadResult:
                if (started && ++polls < 3) return [(byte)PtbResult.Busy];
                started = false;
                var result = new byte[21];
                result[1] = (byte)((header[10] != 0 ? 1 : 0) | (header[71] != 0 ? 2 : 0) | (header[27] != 0 ? 4 : 0) | (header[3] != 0 ? 8 : 0));
                BinaryPrimitives.WriteUInt16LittleEndian(result.AsSpan(2, 2), 100);
                BinaryPrimitives.WriteUInt16LittleEndian(result.AsSpan(4, 2), 99);
                result[6] = unchecked((byte)-4); result[7] = unchecked((byte)-90); result[8] = 2; result[9] = 0x1F;
                BinaryPrimitives.WriteUInt32LittleEndian(result.AsSpan(11, 4), 0x02002000);
                BinaryPrimitives.WriteUInt32LittleEndian(result.AsSpan(16, 4), 0x0207E000);
                return result;
            default: return [(byte)PtbResult.UnknownCommand];
        }
    }
    public Task CloseAsync() { IsOpen = false; return Task.CompletedTask; }
    public ValueTask DisposeAsync() { disposed = true; IsOpen = false; transfer?.Dispose(); replies.Writer.TryComplete(); return ValueTask.CompletedTask; }
}
