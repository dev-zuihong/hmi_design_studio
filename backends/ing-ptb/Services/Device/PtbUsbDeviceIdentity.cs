namespace IngPtbTool.Services.Device;

public static class PtbUsbDeviceIdentity
{
    public const ushort VendorId = 0x36B0;
    public const ushort ProductId = 0x5320;
    private const string VendorIdPathMarker = "vid_36b0";
    private const string ProductIdPathMarker = "pid_5320";

    public static bool MatchesInterfacePath(string? devicePath)
    {
        if (string.IsNullOrWhiteSpace(devicePath)) return false;

        // Device-interface notifications expose VID/PID in the symbolic path;
        // use both values to ignore unrelated USB devices.
        return devicePath.Contains(VendorIdPathMarker, StringComparison.OrdinalIgnoreCase) &&
               devicePath.Contains(ProductIdPathMarker, StringComparison.OrdinalIgnoreCase);
    }
}
