using System.ComponentModel.DataAnnotations;

namespace Jarvis.Application.Configuration;

public sealed class JarvisOptions
{
    public const string Section = "Jarvis";

    public string PublicBaseUrl { get; set; } = "https://localhost";

    [Range(1, 10_000)]
    public int MaxDevicesPerOwner { get; set; } = 100;

    /// <summary>Pepper para HMAC de secretos de devices (≥ 32 bytes). Normalmente vía <c>DeviceSecretPepperFile</c>.</summary>
    [Required]
    [MinLength(32)]
    public string DeviceSecretPepper { get; set; } = string.Empty;

    public string[] CorsOrigins { get; set; } = [];

    /// <summary>Redes (CIDR) desde las que se aceptan X-Forwarded-*.</summary>
    public string[] TrustedProxyNetworks { get; set; } = [];
}

public sealed class MqttOptions
{
    public const string Section = "Jarvis:Mqtt";

    [Required]
    public string Host { get; set; } = "localhost";

    [Range(1, 65535)]
    public int Port { get; set; } = 1883;

    public bool UseTls { get; set; }

    [Required]
    public string Username { get; set; } = "jarvis-cloud";

    [Required]
    public string Password { get; set; } = string.Empty;

    public string InstanceId { get; set; } = Environment.MachineName.ToLowerInvariant();

    /// <summary>Host/puerto públicos devueltos a los devices al registrarse.</summary>
    [Required]
    public string PublicHost { get; set; } = "localhost";

    [Range(1, 65535)]
    public int PublicPort { get; set; } = 8883;

    [Range(1, 60)]
    public int AdminTimeoutSeconds { get; set; } = 5;

    [Range(1, 86_400 * 7)]
    public int SessionExpirySeconds { get; set; } = 86_400;

    [Range(1, 64)]
    public int MaxConcurrentMessages { get; set; } = 4;
}

public sealed class JwtOptions
{
    public const string Section = "Jwt";

    public string Issuer { get; set; } = "jarvis-cloud";

    public string Audience { get; set; } = "jarvis-api";

    [Required]
    [MinLength(32)]
    public string SigningKey { get; set; } = string.Empty;

    public string? PreviousSigningKey { get; set; }

    [Range(1, 1440)]
    public int UserAccessTokenMinutes { get; set; } = 15;

    [Range(1, 1440)]
    public int DeviceAccessTokenMinutes { get; set; } = 10;

    [Range(1, 365)]
    public int RefreshTokenDays { get; set; } = 30;

    [Range(0, 300)]
    public int ClockSkewSeconds { get; set; } = 30;
}

public sealed class TaskOptions
{
    public const string Section = "Jarvis:Tasks";

    [Range(1, 3600)]
    public int AckTimeoutSeconds { get; set; } = 30;

    [Range(1, 100)]
    public int MaxAttempts { get; set; } = 5;

    [Range(1, 3600)]
    public int DispatchIntervalSeconds { get; set; } = 5;

    [Range(1, 3600)]
    public int DeliveryTimeoutSweepSeconds { get; set; } = 10;

    [Range(1, 3600)]
    public int ExpirySweepSeconds { get; set; } = 15;

    [Range(1, 3600)]
    public int CancelResendSeconds { get; set; } = 60;

    [Range(1, 1000)]
    public int DispatchBatchSize { get; set; } = 50;

    [Range(1, 10_000)]
    public int CreateLimitPerMinute { get; set; } = 30;
}

public sealed class PresenceOptions
{
    public const string Section = "Jarvis:Presence";

    [Range(1, 3600)]
    public int HeartbeatIntervalSeconds { get; set; } = 30;

    [Range(1, 3600)]
    public int StaleAfterSeconds { get; set; } = 75;

    [Range(1, 86_400)]
    public int OfflineAfterSeconds { get; set; } = 180;

    [Range(1, 3600)]
    public int SweepIntervalSeconds { get; set; } = 15;
}

public sealed class SecurityOptions
{
    public const string Section = "Jarvis:Security";

    [Range(1, 10_000)]
    public int MqttMessagesPerSecond { get; set; } = 20;

    [Range(1, 3600)]
    public int MessageTimestampToleranceSeconds { get; set; } = 300;

    [Range(1, 3600)]
    public int PrincipalStatusCacheSeconds { get; set; } = 30;

    [Range(1, 3600)]
    public int FloodViolationAfterSeconds { get; set; } = 60;

    /// <summary>Límite aplicativo de <c>/api/v1/auth/*</c> por IP (nginx aplica además 10/min).</summary>
    [Range(1, 100_000)]
    public int AuthRequestsPerMinute { get; set; } = 20;

    /// <summary>Peticiones de sincronización y memoria por minuto por principal (sync-protocol.md §10).</summary>
    [Range(1, 100_000)]
    public int SyncRequestsPerMinute { get; set; } = 120;
}

public sealed class RetentionOptions
{
    public const string Section = "Jarvis:Retention";

    public bool Enabled { get; set; } = true;

    [Range(1, 3650)]
    public int EventsDays { get; set; } = 180;

    [Range(1, 3650)]
    public int RefreshTokensDays { get; set; } = 30;

    [Range(100, 100_000)]
    public int BatchSize { get; set; } = 5000;

    [Range(1, 1440)]
    public int IntervalMinutes { get; set; } = 1440;
}

public sealed class BrokerReconciliationOptions
{
    public const string Section = "Jarvis:BrokerReconciliation";

    [Range(1, 1440)]
    public int IntervalMinutes { get; set; } = 60;

    [Range(1, 3600)]
    public int RetrySeconds { get; set; } = 30;
}
