using System.Text;
using System.Text.RegularExpressions;

namespace Jarvis.Domain.Common;

/// <summary>Reglas de formato y límites compartidos por el dominio (ver docs/02-domain).</summary>
public static partial class DomainRules
{
    public const int MaxPayloadBytes = 16 * 1024;
    public const int MaxResultBytes = 32 * 1024;
    public const int MaxCapabilities = 32;
    public const int MaxSupportedTaskTypes = 64;
    public const int MaxDeviceNameLength = 64;
    public const int MaxIdempotencyKeyLength = 100;
    public const int MaxProgressMessageLength = 200;
    public const int MaxErrorMessageLength = 500;
    public const int MaxErrorCodeLength = 64;
    public const int MaxStatusReasonLength = 100;
    public const int MaxSessionIdLength = 64;
    public const int DefaultMaxAttempts = 5;
    public const int MinExpiresInSeconds = 10;
    public const int MaxExpiresInSeconds = 7 * 24 * 3600;
    public const int DefaultExpiresInSeconds = 600;
    public const int MaxDurationSeconds = 86_400;

    // ---- Firma end-to-end (ADR-012) ---------------------------------------------
    /// <summary>Único algoritmo aceptado. El Cloud no verifica la firma; solo comprueba su forma.</summary>
    public const string SignatureAlgorithm = "Ed25519";
    public const int MaxKeyIdLength = 64;
    public const int Ed25519SignatureBytes = 64;
    public const int MaxNonceLength = 128;
    /// <summary>Ventana de frescura de una decisión de aprobación, alineada con la del agente.</summary>
    public const int ApprovalDecisionMaxAgeSeconds = 300;

    [GeneratedRegex("^[A-Z][A-Z0-9_]{1,63}$")]
    private static partial Regex TaskTypeRegex();

    [GeneratedRegex("^[a-z][a-z0-9-]{1,31}$")]
    private static partial Regex CapabilityRegex();

    [GeneratedRegex("^[A-Za-z0-9_.:-]{1,100}$")]
    private static partial Regex IdempotencyKeyRegex();

    [GeneratedRegex("^[A-Za-z0-9_.:-]{1,64}$")]
    private static partial Regex KeyIdRegex();

    [GeneratedRegex("^[A-Za-z0-9_.:-]{1,128}$")]
    private static partial Regex NonceRegex();

    public static bool IsValidTaskType(string? value) => value is not null && TaskTypeRegex().IsMatch(value);

    public static bool IsValidCapability(string? value) => value is not null && CapabilityRegex().IsMatch(value);

    public static bool IsValidIdempotencyKey(string? value) => value is not null && IdempotencyKeyRegex().IsMatch(value);

    public static bool IsValidDeviceName(string? value) =>
        !string.IsNullOrWhiteSpace(value) && value.Length <= MaxDeviceNameLength && value.Trim() == value;

    public static bool IsValidKeyId(string? value) => value is not null && KeyIdRegex().IsMatch(value);

    public static bool IsValidNonce(string? value) => value is not null && NonceRegex().IsMatch(value);

    /// <summary>
    /// Comprueba la <b>forma</b> de una firma: algoritmo, id de clave y 64 bytes en base64.
    /// Verificar que la firma es auténtica es trabajo del device destino (ADR-012): el Cloud no
    /// tiene —ni debe tener— la clave pública.
    /// </summary>
    public static bool IsWellFormedSignature(string? alg, string? keyId, string? value)
    {
        if (!string.Equals(alg, SignatureAlgorithm, StringComparison.OrdinalIgnoreCase) || !IsValidKeyId(keyId))
        {
            return false;
        }

        Span<byte> buffer = stackalloc byte[Ed25519SignatureBytes + 4];
        return value is not null
            && Convert.TryFromBase64String(value, buffer, out var written)
            && written == Ed25519SignatureBytes;
    }

    public static int Utf8Length(string value) => Encoding.UTF8.GetByteCount(value);

    public static string Truncate(string? value, int max) =>
        string.IsNullOrEmpty(value) ? string.Empty : value.Length <= max ? value : value[..max];
}
