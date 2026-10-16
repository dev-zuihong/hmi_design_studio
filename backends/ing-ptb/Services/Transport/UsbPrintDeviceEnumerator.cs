using System.ComponentModel;
using System.Runtime.InteropServices;
using IngPtbTool.Models;
using IngPtbTool.Services.Device;

namespace IngPtbTool.Services.Transport;

public sealed class UsbPrintDeviceEnumerator : IPtbDeviceEnumerator
{
    private static readonly Guid UsbPrintInterface = new("28D78FAD-5A12-11D1-AE5B-0000F803A8C2");

    public IReadOnlyList<PtbDeviceInfo> Enumerate()
    {
        if (!OperatingSystem.IsWindows()) return [];
        List<PtbDeviceInfo> devices = [];
        Guid interfaceGuid = UsbPrintInterface;
        IntPtr set = Native.SetupDiGetClassDevs(ref interfaceGuid, null, IntPtr.Zero, 0x12);
        if (set == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error());
        try
        {
            for (uint index = 0; ; index++)
            {
                Native.SP_DEVICE_INTERFACE_DATA interfaceData = new() { cbSize = Marshal.SizeOf<Native.SP_DEVICE_INTERFACE_DATA>() };
                if (!Native.SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref interfaceGuid, index, ref interfaceData))
                {
                    int error = Marshal.GetLastWin32Error();
                    if (error == 259) break;
                    throw new Win32Exception(error);
                }

                Native.SP_DEVINFO_DATA devInfo = new() { cbSize = Marshal.SizeOf<Native.SP_DEVINFO_DATA>() };
                Native.SetupDiGetDeviceInterfaceDetail(set, ref interfaceData, IntPtr.Zero, 0, out uint required, ref devInfo);
                IntPtr detail = Marshal.AllocHGlobal(checked((int)required));
                try
                {
                    Marshal.WriteInt32(detail, IntPtr.Size == 8 ? 8 : 6);
                    if (!Native.SetupDiGetDeviceInterfaceDetail(set, ref interfaceData, detail, required, out _, ref devInfo))
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                    string path = Marshal.PtrToStringUni(detail + 4) ?? string.Empty;
                    if (!PtbUsbDeviceIdentity.MatchesInterfacePath(path)) continue;
                    devices.Add(new PtbDeviceInfo(path, GetLocation(set, ref devInfo)));
                }
                finally { Marshal.FreeHGlobal(detail); }
            }
        }
        finally { Native.SetupDiDestroyDeviceInfoList(set); }

        return devices.OrderBy(x => x.DevicePath, StringComparer.OrdinalIgnoreCase).ToArray();
    }

    private static string GetLocation(IntPtr set, ref Native.SP_DEVINFO_DATA info)
    {
        char[] value = new char[512];
        return Native.SetupDiGetDeviceRegistryProperty(set, ref info, 0x0000000D, out _, value, value.Length * 2, out _)
            ? new string(value, 0, Array.IndexOf(value, '\0') is int length and >= 0 ? length : value.Length)
            : string.Empty;
    }

    private static class Native
    {
        [StructLayout(LayoutKind.Sequential)]
        internal struct SP_DEVICE_INTERFACE_DATA { public int cbSize; public Guid InterfaceClassGuid; public int Flags; public IntPtr Reserved; }
        [StructLayout(LayoutKind.Sequential)]
        internal struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public uint DevInst; public IntPtr Reserved; }

        [DllImport("setupapi.dll", EntryPoint = "SetupDiGetClassDevsW", SetLastError = true, CharSet = CharSet.Unicode)]
        internal static extern IntPtr SetupDiGetClassDevs(ref Guid classGuid, string? enumerator, IntPtr parent, uint flags);
        [DllImport("setupapi.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool SetupDiEnumDeviceInterfaces(IntPtr set, IntPtr info, ref Guid guid, uint index, ref SP_DEVICE_INTERFACE_DATA data);
        [DllImport("setupapi.dll", EntryPoint = "SetupDiGetDeviceInterfaceDetailW", SetLastError = true, CharSet = CharSet.Unicode)] [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr set, ref SP_DEVICE_INTERFACE_DATA data, IntPtr detail, uint size, out uint required, ref SP_DEVINFO_DATA info);
        [DllImport("setupapi.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
        [DllImport("setupapi.dll", EntryPoint = "SetupDiGetDeviceRegistryPropertyW", SetLastError = true, CharSet = CharSet.Unicode)] [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool SetupDiGetDeviceRegistryProperty(IntPtr set, ref SP_DEVINFO_DATA info, uint property, out uint type, [Out] char[] value, int size, out uint required);
    }
}
