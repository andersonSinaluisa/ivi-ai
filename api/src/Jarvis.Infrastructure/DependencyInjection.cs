using Jarvis.Application.Abstractions;
using Jarvis.Infrastructure.Mqtt;
using Jarvis.Infrastructure.Persistence;
using Jarvis.Infrastructure.Security;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;

namespace Jarvis.Infrastructure;

public static class DependencyInjection
{
    /// <summary>Solo persistencia (DbContext, SQL de memoria y jobs): lo que necesita Jarvis.Worker.</summary>
    public static IServiceCollection AddJarvisPersistence(this IServiceCollection services, IConfiguration configuration)
    {
        var connectionString = PostgresSetup.ResolvePasswordFile(
            configuration.GetConnectionString("Postgres") ?? throw new InvalidOperationException("ConnectionStrings:Postgres is required."));
        services.AddDbContext<JarvisDbContext>(options => PostgresSetup.Configure(options, connectionString));
        services.AddScoped<IJarvisDbContext>(sp => sp.GetRequiredService<JarvisDbContext>());
        services.AddScoped<IMemorySql, MemorySql>();
        return services;
    }

    public static IServiceCollection AddJarvisInfrastructure(this IServiceCollection services, IConfiguration configuration, bool enableMqtt = true)
    {
        var connectionString = PostgresSetup.ResolvePasswordFile(
            configuration.GetConnectionString("Postgres") ?? throw new InvalidOperationException("ConnectionStrings:Postgres is required."));
        var maintenance = configuration.GetConnectionString("PostgresMaintenance");
        var maintenanceConnection = string.IsNullOrWhiteSpace(maintenance) ? connectionString : PostgresSetup.ResolvePasswordFile(maintenance);

        services.AddDbContext<JarvisDbContext>(options => PostgresSetup.Configure(options, connectionString));
        services.AddScoped<IJarvisDbContext>(sp => sp.GetRequiredService<JarvisDbContext>());
        services.AddScoped<IMemorySql, MemorySql>();
        services.AddSingleton<IRetentionStore>(_ => new RetentionStore(maintenanceConnection));

        services.AddSingleton<IDeviceSecretHasher, HmacDeviceSecretHasher>();
        services.AddSingleton<IUserPasswordHasher, IdentityUserPasswordHasher>();
        services.AddSingleton<ITokenIssuer, JwtTokenIssuer>();
        services.AddSingleton<IIpHasher, DailyIpHasher>();

        services.AddSingleton<MqttCloudConnection>();
        services.AddSingleton<IMqttConnectionState>(sp => sp.GetRequiredService<MqttCloudConnection>());
        services.AddSingleton<IDeviceMessagePublisher, MqttDeviceMessagePublisher>();
        services.AddSingleton<MosquittoDynsecAdmin>();
        services.AddSingleton<IBrokerCredentialAdmin>(sp => sp.GetRequiredService<MosquittoDynsecAdmin>());
        if (enableMqtt)
        {
            services.AddHostedService(sp => sp.GetRequiredService<MqttCloudConnection>());
        }

        return services;
    }
}

/// <summary>
/// Resuelve claves <c>XxxFile</c> leyendo el archivo indicado (Docker secrets) y publicándolo como <c>Xxx</c>.
/// Así ningún secreto viaja en variables de entorno (docs/05-security/secrets-management.md).
/// </summary>
public static class SecretFileConfiguration
{
    public static IConfigurationBuilder AddSecretFiles(this IConfigurationBuilder builder)
    {
        var current = builder.Build();
        var resolved = new Dictionary<string, string?>(StringComparer.OrdinalIgnoreCase);
        foreach (var (key, value) in current.AsEnumerable())
        {
            if (value is null || !key.EndsWith("File", StringComparison.Ordinal) || key.EndsWith(":File", StringComparison.Ordinal))
            {
                continue;
            }

            var target = key[..^"File".Length];
            if (!string.IsNullOrEmpty(current[target]))
            {
                continue;
            }

            if (!File.Exists(value))
            {
                throw new InvalidOperationException($"Secret file for '{target}' not found.");
            }

            resolved[target] = File.ReadAllText(value).Trim();
        }

        return resolved.Count == 0 ? builder : builder.AddInMemoryCollection(resolved);
    }
}
