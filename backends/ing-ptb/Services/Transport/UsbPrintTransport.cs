using Microsoft.Win32.SafeHandles;
using System.ComponentModel;
using System.Runtime.InteropServices;
using IngPtbTool.Models;

namespace IngPtbTool.Services.Transport;

public sealed class UsbPrintTransport(PtbDeviceInfo device, string owner = "", bool resetConfirmed = false) : IPtbTransport
{
    private FileStream? readStream;
    private FileStream? writeStream;
    private SafeFileHandle? zeroPacketHandle;
    private readonly SemaphoreSlim writeGate = new(1, 1);
    private bool disposed;
    private UsbDeviceLease? lease;
    private const int BulkMaxPacketSize = 64;
    public PtbDeviceInfo Device { get; } = device;
    public bool IsOpen => readStream is not null && writeStream is not null;
    public UsbRecoveryGuard Recovery { get; } = new(device.DevicePath, owner);

    public Task OpenAsync(CancellationToken cancellationToken)
    {
        if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException();
        ObjectDisposedException.ThrowIf(disposed, this);
        if (IsOpen) return Task.CompletedTask;
        cancellationToken.ThrowIfCancellationRequested();
        lease = UsbDeviceLease.Acquire(Device.DevicePath);
        try {
        Recovery.Check(resetConfirmed);
        SafeFileHandle readHandle = File.OpenHandle(Device.DevicePath, FileMode.Open, FileAccess.Read,
            FileShare.ReadWrite, FileOptions.Asynchronous);
        try
        {
            SafeFileHandle writeHandle = File.OpenHandle(Device.DevicePath, FileMode.Open, FileAccess.Write,
                FileShare.ReadWrite, FileOptions.Asynchronous | FileOptions.WriteThrough);
            SafeFileHandle? zlpHandle = null;
            try
            {
                // FileStream uses overlapped I/O. Keep a separate synchronous handle
                // for the zero-byte request because an overlapped WriteFile requires
                // an OVERLAPPED structure even when nNumberOfBytesToWrite is zero.
                zlpHandle = File.OpenHandle(Device.DevicePath, FileMode.Open,
                    FileAccess.Write, FileShare.ReadWrite, FileOptions.WriteThrough);
                readStream = new FileStream(readHandle, FileAccess.Read, BulkMaxPacketSize, true);
                writeStream = new FileStream(writeHandle, FileAccess.Write, BulkMaxPacketSize, true);
                zeroPacketHandle = zlpHandle;
            }
            catch
            {
                writeHandle.Dispose();
                zlpHandle?.Dispose();
                throw;
            }
        }
        catch
        {
            readHandle.Dispose();
            throw;
        }
        } catch { lease.Dispose(); lease = null; throw; }
        return Task.CompletedTask;
    }

    public ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken) =>
        readStream?.ReadAsync(buffer, cancellationToken) ?? throw new InvalidOperationException("Transport is closed.");

    public async ValueTask WriteAsync(ReadOnlyMemory<byte> data, CancellationToken cancellationToken)
    {
        await writeGate.WaitAsync(cancellationToken);
        try
        {
            FileStream current = writeStream ?? throw new InvalidOperationException("Transport is closed.");
            await current.WriteAsync(data, cancellationToken);
            await current.FlushAsync(cancellationToken);
            if (NeedsZeroLengthPacket(data.Length)) WriteZeroLengthPacket();
        }
        finally { writeGate.Release(); }
    }

    public async Task CloseAsync()
    {
        await writeGate.WaitAsync();
        try
        {
            readStream?.Dispose();
            writeStream?.Dispose();
            zeroPacketHandle?.Dispose();
            readStream = null;
            writeStream = null;
            zeroPacketHandle = null;
            lease?.Dispose(); lease = null;
        }
        finally { writeGate.Release(); }
    }

    public async ValueTask DisposeAsync()
    {
        if (disposed) return;
        disposed = true;
        await CloseAsync();
        writeGate.Dispose();
    }

    private static bool NeedsZeroLengthPacket(int length) =>
        length > 0 && length % BulkMaxPacketSize == 0;

    private void WriteZeroLengthPacket()
    {
        SafeFileHandle handle = zeroPacketHandle ?? throw new InvalidOperationException("Transport is closed.");
        if (!WriteFile(handle, IntPtr.Zero, 0, out uint written, IntPtr.Zero) || written != 0)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "提交 USB Bulk ZLP 失败。");
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool WriteFile(SafeFileHandle fileHandle, IntPtr buffer,
        uint numberOfBytesToWrite, out uint numberOfBytesWritten, IntPtr overlapped);
}
