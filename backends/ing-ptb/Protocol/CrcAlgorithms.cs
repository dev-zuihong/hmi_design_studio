namespace IngPtbTool.Protocol;

/// <summary>与 PTB 固件查表实现等价的 CRC16/Modbus 变体。</summary>
public static class Crc16Ptb
{
    public static ushort Compute(ReadOnlySpan<byte> data)
    {
        ushort crc = 0xFFFF;
        foreach (byte value in data)
        {
            crc ^= value;
            for (int bit = 0; bit < 8; bit++)
                crc = (ushort)(((crc & 1) != 0) ? (crc >> 1) ^ 0xA001 : crc >> 1);
        }

        return (ushort)((crc << 8) | (crc >> 8));
    }
}

/// <summary>用于界面文件核对的 IEEE CRC32。</summary>
public static class Crc32Ieee
{
    public static uint Compute(ReadOnlySpan<byte> data)
    {
        uint crc = 0xFFFFFFFF;
        foreach (byte value in data)
        {
            crc ^= value;
            for (int bit = 0; bit < 8; bit++)
                crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xEDB88320u : crc >> 1;
        }

        return crc ^ 0xFFFFFFFF;
    }
}
