using System.Security.Cryptography;
using System.Text.Json.Nodes;

namespace IngPtb.Backend;

public sealed record ResourceFile(string Id, string Path, string Name, string Sha256, long Size)
{
    public byte[] Read()
    {
        byte[] bytes = File.ReadAllBytes(Path);
        if (bytes.LongLength != Size || Convert.ToHexStringLower(SHA256.HashData(bytes)) != Sha256)
            throw new InvalidDataException($"资源 {Id} 内容已经变化，请重新导入。");
        return bytes;
    }
    public JsonObject Reference() => new() { ["id"] = Id, ["name"] = Name, ["sha256"] = Sha256, ["size"] = Size };
}

public sealed class ResourceRegistry
{
    private readonly System.Collections.Concurrent.ConcurrentDictionary<string, ResourceFile> files = new(StringComparer.Ordinal);
    public void Register(JsonObject? resources)
    {
        if (resources is null) return;
        foreach (var (id, node) in resources)
        {
            if (id.Length is < 1 or > 128 || id.Any(c => !char.IsAsciiLetterOrDigit(c) && c is not ('_' or '-')))
                throw new InvalidDataException("资源ID只能包含ASCII字母、数字、下划线和连字符。");
            if (node is not JsonObject item) throw new InvalidDataException("资源描述必须为对象。");
            string path = item["path"]?.GetValue<string>() ?? throw new InvalidDataException("资源缺少路径。");
            if (!System.IO.Path.IsPathFullyQualified(path)) throw new InvalidDataException("资源路径必须由宿主解析为绝对路径。");
            path = System.IO.Path.GetFullPath(path);
            var info = new FileInfo(path);
            string name = item["name"]?.GetValue<string>() ?? info.Name;
            if (System.Text.Encoding.UTF8.GetByteCount(name) > 512) throw new InvalidDataException("资源文件名超过512字节限制。");
            if (!info.Exists || info.Length > 16 * 1024 * 1024) throw new InvalidDataException("资源不存在或超过16MiB。");
            for (FileSystemInfo? current = info; current is not null; current = current is FileInfo file ? file.Directory : ((DirectoryInfo)current).Parent)
                if ((current.Attributes & FileAttributes.ReparsePoint) != 0) throw new InvalidDataException("资源路径包含重解析点。");
            long? size = item["size"] is JsonNode sizeNode ? long.Parse(sizeNode.ToJsonString(), System.Globalization.CultureInfo.InvariantCulture) : null;
            if (size.HasValue && size.Value != info.Length) throw new InvalidDataException("资源长度校验失败。");
            string? expected = item["sha256"]?.GetValue<string>();
            if (files.TryGetValue(id, out var cached) && cached.Path == path && cached.Size == info.Length
                && expected is not null && cached.Sha256.Equals(expected, StringComparison.OrdinalIgnoreCase)) continue;
            if (!files.ContainsKey(id) && files.Count >= 256) throw new InvalidDataException("资源数量超过256限制。");
            string hash = Convert.ToHexStringLower(SHA256.HashData(File.ReadAllBytes(path)));
            if (expected is not null && !hash.Equals(expected, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("资源SHA256校验失败。");
            if (files.TryGetValue(id, out var previous) && previous.Sha256 != hash) throw new InvalidDataException("已注册的资源ID不能替换内容。");
            files[id] = new(id, path, name, hash, info.Length);
        }
    }
    public ResourceFile Get(string id) => files.TryGetValue(id, out var file) ? file : throw new InvalidDataException($"资源未注册：{id}");
    public string ResolveId(JsonNode? reference)
    {
        string? id = reference is JsonValue scalar && scalar.TryGetValue<string>(out var value) ? value : reference?["id"]?.GetValue<string>() ?? reference?["assetId"]?.GetValue<string>();
        return !string.IsNullOrWhiteSpace(id) && files.ContainsKey(id) ? id : throw new InvalidDataException("请选择已导入的工程文件资源。");
    }
    public string? Find(string value)
    {
        if (string.IsNullOrWhiteSpace(value)) return null;
        var exact = files.Values.Where(file => file.Id == value || file.Path.Equals(value, StringComparison.OrdinalIgnoreCase)).ToArray();
        if (exact.Length == 1) return exact[0].Id;
        var names = files.Values.Where(file => file.Name.Equals(System.IO.Path.GetFileName(value), StringComparison.OrdinalIgnoreCase)).ToArray();
        if (names.Length > 1) throw new InvalidDataException($"同名资源存在歧义，请显式映射：{value}");
        return names.SingleOrDefault()?.Id;
    }
}
