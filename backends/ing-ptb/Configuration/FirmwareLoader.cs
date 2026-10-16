using System.Globalization;
using IngPtbTool.Models;

namespace IngPtbTool.Configuration;

/// <summary>加载 BIN、Intel HEX 和 IHX 固件文件。</summary>
public static class FirmwareLoader
{
    /// <summary>按指定目标地址加载原始 BIN 文件。</summary>
    public static FirmwareSegment LoadBin(string path, uint destinationAddress) =>
        new(Path.GetFileName(path), destinationAddress, File.ReadAllBytes(path), Path.GetFullPath(path));

    /// <summary>解析 Intel HEX/IHX，并按连续地址区间拆分固件段。</summary>
    public static IReadOnlyList<FirmwareSegment> LoadIntelHex(string path) =>
        ParseIntelHex(File.ReadAllText(path), Path.GetFileName(path), Path.GetFullPath(path));

    public static IReadOnlyList<FirmwareSegment> ParseIntelHex(string text, string name, string? sourcePath = null)
    {
        SortedDictionary<uint, byte> bytes = [];
        uint upperAddress = 0;
        int lineNumber = 0;
        foreach (string rawLine in text.Split('\n', StringSplitOptions.RemoveEmptyEntries))
        {
            lineNumber++;
            string line = rawLine.Trim();
            if (line.Length < 11 || line[0] != ':') throw new FormatException($"Invalid HEX line {lineNumber}.");
            byte[] record = Convert.FromHexString(line[1..]);
            int count = record[0];
            if (record.Length != count + 5) throw new FormatException($"Invalid HEX length at line {lineNumber}.");
            int sum = 0;
            foreach (byte value in record) sum = (sum + value) & 0xFF;
            if (sum != 0) throw new FormatException($"Invalid HEX checksum at line {lineNumber}.");
            ushort offset = (ushort)((record[1] << 8) | record[2]);
            byte type = record[3];
            switch (type)
            {
                case 0x00:
                    uint address = checked(upperAddress + offset);
                    for (int i = 0; i < count; i++)
                        if (!bytes.TryAdd(checked(address + (uint)i), record[4 + i]))
                            throw new FormatException($"Overlapping HEX address at line {lineNumber}.");
                    break;
                case 0x01:
                    goto Parsed;
                case 0x02:
                    if (count != 2) throw new FormatException($"Invalid segment address at line {lineNumber}.");
                    upperAddress = (uint)((record[4] << 8) | record[5]) << 4;
                    break;
                case 0x04:
                    if (count != 2) throw new FormatException($"Invalid linear address at line {lineNumber}.");
                    upperAddress = (uint)((record[4] << 8) | record[5]) << 16;
                    break;
                case 0x03:
                case 0x05:
                    break;
                default:
                    throw new FormatException($"Unsupported HEX record type {type:X2} at line {lineNumber}.");
            }
        }

    Parsed:
        if (bytes.Count == 0) throw new FormatException("HEX file contains no data.");
        List<FirmwareSegment> segments = [];
        uint start = 0, previous = 0;
        List<byte> data = [];
        foreach ((uint address, byte value) in bytes)
        {
            if (data.Count == 0 || address != previous + 1)
            {
                if (data.Count != 0) segments.Add(new FirmwareSegment(
                    name, start, data.ToArray(), sourcePath));
                start = address;
                data = [];
            }
            data.Add(value);
            previous = address;
        }
        segments.Add(new FirmwareSegment(name, start, data.ToArray(), sourcePath));
        return segments;
    }

    public static uint ParseAddress(string value)
    {
        string text = value.Trim();
        return text.StartsWith("0x", StringComparison.OrdinalIgnoreCase)
            ? uint.Parse(text[2..], NumberStyles.HexNumber, CultureInfo.InvariantCulture)
            : uint.Parse(text, NumberStyles.HexNumber, CultureInfo.InvariantCulture);
    }
}
