using System.Security.Cryptography;
using System.Text;

namespace IngPtbTool.Services.Transport;

/// <summary>Persists uncertain side effects across provider crashes/restarts.
/// A Windows USB interface lease must be held for every mutation.</summary>
public sealed class UsbRecoveryGuard(string devicePath, string owner)
{
    private readonly string marker = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "HmiDesignStudio", "ing-ptb", "recovery", Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(devicePath.ToUpperInvariant()))) + ".state");
    public void Check(bool resetConfirmed)
    {
        if (resetConfirmed) { Clear(); return; }
        if (File.Exists(marker) && File.ReadAllText(marker) != owner)
            throw new IOException("DEVICE_RESET_REQUIRED: 上一运行实例留下未确认设备事务，请复位设备并确认恢复。");
    }
    public void Mark()
    {
        Directory.CreateDirectory(Path.GetDirectoryName(marker)!);
        string temporary = marker + "." + Guid.NewGuid().ToString("N");
        File.WriteAllText(temporary, owner);
        File.Move(temporary, marker, true);
    }
    public void Clear() { if (File.Exists(marker)) File.Delete(marker); }
}
