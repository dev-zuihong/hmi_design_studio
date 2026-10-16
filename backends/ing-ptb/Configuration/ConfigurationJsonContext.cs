using System.Text.Json.Serialization;

namespace IngPtbTool.Configuration;

[JsonSourceGenerationOptions(
    WriteIndented = true,
    PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase,
    PropertyNameCaseInsensitive = true)]
[JsonSerializable(typeof(BurnConfigurationDocument))]
internal sealed partial class ConfigurationJsonContext : JsonSerializerContext;
