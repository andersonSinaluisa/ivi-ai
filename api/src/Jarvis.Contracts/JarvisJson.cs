using System.Text.Json;
using System.Text.Json.Serialization;

namespace Jarvis.Contracts;

/// <summary>Opciones JSON del protocolo: camelCase, enums como string, nulos omitidos, campos desconocidos ignorados.</summary>
public static class JarvisJson
{
    public static JsonSerializerOptions Options { get; } = Create();

    public static JsonSerializerOptions Create()
    {
        var options = new JsonSerializerOptions(JsonSerializerDefaults.Web)
        {
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            UnmappedMemberHandling = JsonUnmappedMemberHandling.Skip,
        };
        options.Converters.Add(new JsonStringEnumConverter());
        return options;
    }

    public static JsonElement ToElement<T>(T value) => JsonSerializer.SerializeToElement(value, Options);

    public static JsonElement ParseElement(string json)
    {
        using var document = JsonDocument.Parse(json);
        return document.RootElement.Clone();
    }

    public static JsonElement EmptyObject { get; } = ParseElement("{}");
}
