namespace Jarvis.Domain.Common;

/// <summary>Códigos que el dominio usa en <see cref="DomainException"/>; coinciden con el catálogo de errores de la API.</summary>
public static class DomainErrorCodes
{
    public const string ValidationFailed = "VALIDATION_FAILED";
    public const string InvalidStateTransition = "INVALID_STATE_TRANSITION";
    public const string PayloadTooLarge = "PAYLOAD_TOO_LARGE";
    public const string DeviceRevoked = "DEVICE_REVOKED";
}
