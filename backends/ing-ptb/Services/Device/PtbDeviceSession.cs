using System.Buffers.Binary;
using IngPtbTool.Models;
using IngPtbTool.Protocol;
using IngPtbTool.Services.Transport;

namespace IngPtbTool.Services.Device;

/// <summary>A device has exactly one outstanding wire transaction. Caller cancellation
/// stops admission, but never abandons a request that may already have reached USB.</summary>
public sealed class PtbDeviceSession(IPtbTransport transport) : IAsyncDisposable
{
    private readonly SemaphoreSlim requestGate = new(1, 1);
    private readonly object gate = new();
    private PtbFrameStreamParser parser = new();
    private CancellationTokenSource? readerStop;
    private Task? reader;
    private TaskCompletionSource<PtbFrame>? pending;
    private PtbCommand pendingCommand;
    public IPtbTransport Transport { get; } = transport;
    public bool Faulted { get; private set; }
    public bool RequiresDeviceReset { get; private set; }
    public event Action<PtbFrame>? Unsolicited;
    public event Action<string>? Lost;
    public event Action<bool, byte[]>? Trace;

    public async Task ConnectAsync(CancellationToken cancellation)
    {
        if (RequiresDeviceReset) throw new InvalidOperationException("DEVICE_RESET_REQUIRED: 上次传输结果不确定，请先复位设备再恢复连接。");
        if (Transport.IsOpen) return;
        await Transport.OpenAsync(cancellation);
        parser = new PtbFrameStreamParser();
        Faulted = false;
        readerStop = new CancellationTokenSource();
        reader = ReadLoopAsync(readerStop.Token);
    }

    public async Task<PtbFrame> RequestAsync(PtbCommand command, byte[] payload, TimeSpan timeout, CancellationToken cancellation)
    {
        await requestGate.WaitAsync(cancellation);
        try
        {
            cancellation.ThrowIfCancellationRequested();
            if (Faulted || !Transport.IsOpen) throw new IOException("DEVICE_FAULT: 连接不可用，请重新握手。");
            var waiter = new TaskCompletionSource<PtbFrame>(TaskCreationOptions.RunContinuationsAsynchronously);
            lock (gate) { pendingCommand = command; pending = waiter; }
            // Timeout is owned by the transaction, not by the requesting UI/job token.
            using var deadline = new CancellationTokenSource(timeout);
            try
            {
                byte[] bytes = PtbFrameCodec.Encode(new PtbFrame(PtbFrame.HostType, command, payload));
                Trace?.Invoke(true, bytes);
                await Transport.WriteAsync(bytes, deadline.Token);
                PtbFrame response = await waiter.Task.WaitAsync(deadline.Token);
                if (response.Payload.Length == 0) throw new InvalidDataException("应答缺少结果码。");
                if (response.Payload[0] == 0)
                {
                    int expected = command switch { PtbCommand.GetVersion => 5, PtbCommand.DeviceId when payload.FirstOrDefault() == 0 => 2,
                        PtbCommand.GetConfiguration => 95, PtbCommand.ReadResult => 21, _ => 1 };
                    if (response.Payload.Length != expected) throw new InvalidDataException($"{command}应答长度无效。");
                }
                return response;
            }
            catch (Exception error)
            {
                Faulted = true;
                RequiresDeviceReset |= command is PtbCommand.Start or PtbCommand.Configure or PtbCommand.ConfigureDutFirmware
                    || command == PtbCommand.DeviceId && payload.FirstOrDefault() == (byte)PtbDeviceIdOperation.Set;
                readerStop?.Cancel();
                await Transport.CloseAsync();
                string message = error is OperationCanceledException ? $"DEVICE_TIMEOUT: {command} 应答超时；未自动重发。" : error.Message;
                Lost?.Invoke(message);
                throw new IOException(message, error);
            }
            finally { lock (gate) pending = null; }
        }
        finally { requestGate.Release(); }
    }

    public async Task<string> GetVersionAsync(CancellationToken token)
    {
        var frame = await RequestAsync(PtbCommand.GetVersion, [], TimeSpan.FromSeconds(1), token);
        EnsureSuccess(frame);
        if (frame.Payload.Length != 5) throw new InvalidDataException("C5 版本应答长度无效。");
        return $"{BinaryPrimitives.ReadUInt16LittleEndian(frame.Payload.AsSpan(1, 2))}.{frame.Payload[3]}.{frame.Payload[4]}";
    }
    public async Task<byte> GetDeviceIdAsync(CancellationToken token)
    {
        var frame = await RequestAsync(PtbCommand.DeviceId, [0], TimeSpan.FromSeconds(1), token);
        EnsureSuccess(frame);
        if (frame.Payload.Length != 2) throw new InvalidDataException("C4 ID 应答长度无效。");
        return frame.Payload[1];
    }
    public static void EnsureSuccess(PtbFrame frame)
    {
        if (frame.Type != PtbFrame.DeviceType || frame.Payload.Length == 0) throw new InvalidDataException("无效设备应答。");
        if (frame.Payload[0] != 0) throw new InvalidOperationException($"{frame.Command}: {((PtbResult)frame.Payload[0]).ToDisplayText()}");
    }

    private async Task ReadLoopAsync(CancellationToken token)
    {
        var bytes = new byte[4096];
        try
        {
            while (!token.IsCancellationRequested)
            {
                int count = await Transport.ReadAsync(bytes, token);
                if (count == 0) throw new EndOfStreamException("USB Printer 设备已断开。");
                Trace?.Invoke(false, bytes.AsSpan(0, count).ToArray());
                foreach (var frame in parser.Append(bytes.AsSpan(0, count)))
                {
                    bool matched;
                    lock (gate) matched = frame.Type == PtbFrame.DeviceType && frame.Command == pendingCommand && pending?.TrySetResult(frame) == true;
                    if (!matched && frame.Type == PtbFrame.DeviceType) Unsolicited?.Invoke(frame);
                }
            }
        }
        catch (Exception) when (token.IsCancellationRequested) { }
        catch (Exception error)
        {
            Faulted = true;
            lock (gate)
            {
                if (pending is not null)
                {
                    RequiresDeviceReset |= pendingCommand is PtbCommand.Start or PtbCommand.Configure or PtbCommand.ConfigureDutFirmware;
                    pending.TrySetException(error);
                }
            }
            Lost?.Invoke(error.Message);
        }
    }

    public async ValueTask DisposeAsync()
    {
        // Wait for an admitted transaction to finish/timeout before closing transport.
        await requestGate.WaitAsync();
        try
        {
            readerStop?.Cancel();
            await Transport.CloseAsync();
            if (reader is not null) try { await reader; } catch { }
            readerStop?.Dispose(); readerStop = null;
            await Transport.DisposeAsync();
        }
        finally { requestGate.Release(); }
    }
}
