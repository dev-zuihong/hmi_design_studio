using System.Security.Cryptography;
using System.Text;

namespace IngPtbTool.Services.Transport;

/// <summary>A dedicated thread owns the Windows mutex for its complete lifetime.
/// Multiple processes cannot interleave C3/C7 on the same physical interface.</summary>
internal sealed class UsbDeviceLease : IDisposable
{
    private readonly ManualResetEventSlim release = new(false);
    private readonly Thread owner;
    private bool disposed;
    private UsbDeviceLease(string path, TaskCompletionSource<bool> ready)
    {
        string name = "Global\\HmiPtbUsb_" + Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(path.ToUpperInvariant())));
        owner = new Thread(() =>
        {
            using var mutex = new Mutex(false, name);
            bool acquired = false;
            try
            {
                try { acquired = mutex.WaitOne(0); }
                catch (AbandonedMutexException) { acquired = true; }
                if (!acquired) { ready.TrySetException(new IOException("DEVICE_IN_USE: 该设备正在被其他运行实例使用。")); return; }
                ready.TrySetResult(true);
                release.Wait();
            }
            catch (Exception error) { ready.TrySetException(error); }
            finally { if (acquired) mutex.ReleaseMutex(); }
        }) { IsBackground = true, Name = "PTB USB device lease" };
        owner.Start();
    }
    public static UsbDeviceLease Acquire(string path)
    {
        var ready = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var lease = new UsbDeviceLease(path, ready);
        try { ready.Task.GetAwaiter().GetResult(); return lease; }
        catch { lease.Dispose(); throw; }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; release.Set(); owner.Join(); release.Dispose();
    }
}
