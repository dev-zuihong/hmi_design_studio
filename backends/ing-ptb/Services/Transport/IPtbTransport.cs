using IngPtbTool.Models;

namespace IngPtbTool.Services.Transport;

public interface IPtbDeviceEnumerator
{
    IReadOnlyList<PtbDeviceInfo> Enumerate();
}

public interface IPtbTransport : IAsyncDisposable
{
    PtbDeviceInfo Device { get; }
    bool IsOpen { get; }
    Task OpenAsync(CancellationToken cancellationToken);
    ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken);
    ValueTask WriteAsync(ReadOnlyMemory<byte> data, CancellationToken cancellationToken);
    Task CloseAsync();
}
