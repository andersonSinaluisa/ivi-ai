using System.Net;
using System.Text.Json.Serialization;
using System.Threading.RateLimiting;
using Jarvis.Api.Background;
using Jarvis.Api.Endpoints;
using Jarvis.Api.Health;
using Jarvis.Api.Http;
using Jarvis.Api.Realtime;
using Jarvis.Application;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Realtime;
using Jarvis.Application.Security;
using Jarvis.Contracts;
using Jarvis.Infrastructure;
using Jarvis.Infrastructure.Security;
using Microsoft.AspNetCore.Authentication.JwtBearer;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Diagnostics;
using Microsoft.AspNetCore.Diagnostics.HealthChecks;
using Microsoft.AspNetCore.Http.Connections;
using Microsoft.AspNetCore.HttpOverrides;
using Microsoft.Extensions.Diagnostics.HealthChecks;
using Microsoft.Extensions.Options;
using Microsoft.IdentityModel.Tokens;
using OpenTelemetry.Metrics;

namespace Jarvis.Api;

/// <summary>Composición de la API. También la usan los subcomandos y los tests de integración.</summary>
public static class JarvisApiHost
{
    public const int MaxRequestBodyBytes = 64 * 1024;

    /// <summary>Única excepción al límite de 64 KiB: lotes de sincronización (≤ 200 cambios, ≤ 1 MiB).</summary>
    public const int MaxSyncPushBodyBytes = Contracts.Sync.SyncProtocol.MaxPushBytes;

    public const string SyncPushPath = "/api/v1/sync/push";

    public static long BodyLimitFor(PathString path) =>
        path.Equals(SyncPushPath, StringComparison.OrdinalIgnoreCase) ? MaxSyncPushBodyBytes : MaxRequestBodyBytes;

    public static WebApplicationBuilder CreateBuilder(
        string[] args,
        bool enableBackgroundServices = true,
        Action<IConfigurationBuilder>? configure = null)
    {
        var builder = WebApplication.CreateBuilder(args);
        configure?.Invoke(builder.Configuration);
        builder.Configuration.AddSecretFiles();
        ConfigureServices(builder, enableBackgroundServices);
        return builder;
    }

    public static void ConfigureServices(WebApplicationBuilder builder, bool enableBackgroundServices)
    {
        var services = builder.Services;
        var configuration = builder.Configuration;

        builder.WebHost.ConfigureKestrel(kestrel =>
        {
            kestrel.AddServerHeader = false;
            // El máximo de Kestrel es el del push de sincronización; RequestSizeLimitMiddleware baja a 64 KiB el resto.
            kestrel.Limits.MaxRequestBodySize = MaxSyncPushBodyBytes;
            kestrel.Limits.MaxRequestHeadersTotalSize = 32 * 1024;
        });

        if (configuration.GetValue("Logging:Console:FormatterName", "json") == "json")
        {
            builder.Logging.ClearProviders();
            builder.Logging.AddJsonConsole(options =>
            {
                options.IncludeScopes = true;
                options.UseUtcTimestamp = true;
                options.TimestampFormat = "yyyy-MM-ddTHH:mm:ss.fffZ";
            });
        }

        services.AddJarvisApplication(configuration);
        services.AddJarvisInfrastructure(configuration, enableMqtt: enableBackgroundServices);
        services.AddSingleton<IRealtimeNotifier, SignalRRealtimeNotifier>();

        services.ConfigureHttpJsonOptions(options =>
        {
            options.SerializerOptions.Converters.Add(new JsonStringEnumConverter());
        });
        services.AddProblemDetails();
        services.AddExceptionHandler<UnhandledExceptionHandler>();

        ConfigureAuthentication(services, configuration);
        ConfigureRateLimiting(services);
        ConfigureForwardedHeaders(services, configuration);

        var origins = configuration.GetSection("Jarvis:CorsOrigins").Get<string[]>() ?? [];
        services.AddCors(options => options.AddDefaultPolicy(policy =>
        {
            if (origins.Length > 0)
            {
                policy.WithOrigins(origins).AllowAnyHeader().AllowAnyMethod().AllowCredentials();
            }
        }));

        services.AddSignalR(options => options.MaximumReceiveMessageSize = 4 * 1024)
            .AddJsonProtocol(options => options.PayloadSerializerOptions.Converters.Add(new JsonStringEnumConverter()));

        services.AddHealthChecks()
            .AddCheck<PostgresHealthCheck>("postgres", HealthStatus.Unhealthy, ["ready"])
            .AddCheck<MqttHealthCheck>("mqtt", HealthStatus.Degraded, ["ready"])
            .AddCheck<BrokerReconciliationHealthCheck>("broker-reconciliation", HealthStatus.Degraded, ["ready"])
            .AddCheck<BackgroundServicesHealthCheck>("background-services", HealthStatus.Degraded, ["ready"]);

        services.AddOpenTelemetry().WithMetrics(metrics => metrics
            .AddMeter(JarvisMetrics.MeterName)
            .AddAspNetCoreInstrumentation()
            .AddRuntimeInstrumentation()
            .AddPrometheusExporter());

        services.AddOpenApi();

        if (enableBackgroundServices)
        {
            services.AddHostedService<PostCommitEventPump>();
            services.AddHostedService<TaskDispatcherService>();
            services.AddHostedService<DeliveryTimeoutSweeperService>();
            services.AddHostedService<TaskExpirySweeperService>();
            services.AddHostedService<PresenceSweeperService>();
            services.AddHostedService<BrokerReconciliationHostedService>();
            services.AddHostedService<RetentionHostedService>();
        }
    }

    public static WebApplication BuildApp(WebApplicationBuilder builder)
    {
        var app = builder.Build();
        app.UseMiddleware<MetricsPortMiddleware>();
        app.UseForwardedHeaders();
        app.UseExceptionHandler();
        app.UseStatusCodePages(async context =>
        {
            var response = context.HttpContext.Response;
            if (response.StatusCode == StatusCodes.Status404NotFound && !response.HasStarted && response.ContentLength is null)
            {
                await Problems.WriteAsync(context.HttpContext, ErrorCodes.NotFound, "Resource not found.");
            }
        });
        app.UseMiddleware<CorrelationMiddleware>();
        app.UseMiddleware<RequestSizeLimitMiddleware>();
        app.UseCors();
        app.UseAuthentication();
        app.UseMiddleware<PrincipalContextMiddleware>();
        app.UseRateLimiter();
        app.UseAuthorization();

        app.MapHealthChecks("/health/live", new HealthCheckOptions { Predicate = _ => false, ResponseWriter = HealthResponseWriter.WriteAsync })
            .AllowAnonymous();
        app.MapHealthChecks("/health/ready", new HealthCheckOptions { Predicate = c => c.Tags.Contains("ready"), ResponseWriter = HealthResponseWriter.WriteAsync })
            .AllowAnonymous();
        app.MapPrometheusScrapingEndpoint("/metrics");
        app.MapJarvisApi();
        app.MapHub<EventsHub>(EventsHub.Path, options =>
        {
            options.Transports = HttpTransportType.WebSockets | HttpTransportType.LongPolling;
            options.CloseOnAuthenticationExpiration = true;
        });
        if (app.Environment.IsDevelopment())
        {
            app.MapOpenApi();
        }

        return app;
    }

    private static void ConfigureAuthentication(IServiceCollection services, IConfiguration configuration)
    {
        services.AddSingleton<IAuthorizationMiddlewareResultHandler, ProblemAuthorizationResultHandler>();
        services.AddAuthentication(JwtBearerDefaults.AuthenticationScheme).AddJwtBearer();
        services.AddOptions<JwtBearerOptions>(JwtBearerDefaults.AuthenticationScheme)
            .Configure<IOptions<JwtOptions>>((options, jwtOptions) =>
            {
                var jwt = jwtOptions.Value;
                var keys = new List<SecurityKey> { JwtTokenIssuer.CreateKey(jwt.SigningKey, "current") };
                if (!string.IsNullOrEmpty(jwt.PreviousSigningKey))
                {
                    keys.Add(JwtTokenIssuer.CreateKey(jwt.PreviousSigningKey, "previous"));
                }

                options.MapInboundClaims = false;
                options.TokenValidationParameters = new TokenValidationParameters
                {
                    ValidIssuer = jwt.Issuer,
                    ValidAudience = jwt.Audience,
                    IssuerSigningKeys = keys,
                    ValidAlgorithms = [SecurityAlgorithms.HmacSha256],
                    ClockSkew = TimeSpan.FromSeconds(jwt.ClockSkewSeconds),
                    NameClaimType = "sub",
                };
                options.Events = new JwtBearerEvents
                {
                    OnMessageReceived = context =>
                    {
                        // WebSocket no permite headers desde el navegador: access_token solo para el hub.
                        if (context.HttpContext.Request.Path.StartsWithSegments(EventsHub.Path, StringComparison.Ordinal)
                            && context.Request.Query.TryGetValue("access_token", out var token))
                        {
                            context.Token = token;
                        }

                        return Task.CompletedTask;
                    },
                    OnTokenValidated = async context =>
                    {
                        var principal = context.Principal?.ToJarvisPrincipal();
                        if (principal is null)
                        {
                            context.Fail("Invalid token claims.");
                            return;
                        }

                        var services = context.HttpContext.RequestServices;
                        var requestContext = services.GetRequiredService<IRequestContext>();
                        var previous = requestContext.Principal;
                        requestContext.Principal = principal;
                        var active = await services.GetRequiredService<PrincipalStatusService>()
                            .IsActiveAsync(principal, services.GetRequiredService<IJarvisDbContext>(), context.HttpContext.RequestAborted);
                        requestContext.Principal = previous;
                        if (!active)
                        {
                            context.Fail("Principal is not active.");
                        }
                    },
                };
            });

        services.AddAuthorizationBuilder()
            .AddPolicy(Policies.UserOnly, policy => policy.RequireAuthenticatedUser().RequireClaim(JarvisClaims.Type, JarvisClaims.UserType))
            .AddPolicy(Policies.DeviceOnly, policy => policy.RequireAuthenticatedUser().RequireClaim(JarvisClaims.Type, JarvisClaims.DeviceType))
            .AddPolicy(Policies.UserOrDevice, policy => policy.RequireAuthenticatedUser()
                .RequireClaim(JarvisClaims.Type, JarvisClaims.UserType, JarvisClaims.DeviceType));
    }

    private static void ConfigureRateLimiting(IServiceCollection services) =>
        services.AddRateLimiter(options =>
        {
            options.RejectionStatusCode = StatusCodes.Status429TooManyRequests;
            options.AddPolicy(Endpoints.Endpoints.AuthRateLimit, http =>
            {
                var limit = http.RequestServices.GetRequiredService<IOptions<SecurityOptions>>().Value.AuthRequestsPerMinute;
                return RateLimitPartition.GetFixedWindowLimiter(
                    http.Connection.RemoteIpAddress?.ToString() ?? "unknown",
                    _ => new FixedWindowRateLimiterOptions { PermitLimit = limit, Window = TimeSpan.FromMinutes(1), QueueLimit = 0 });
            });
            options.AddPolicy(Endpoints.Endpoints.TaskCreateRateLimit, http =>
            {
                var limit = http.RequestServices.GetRequiredService<IOptions<TaskOptions>>().Value.CreateLimitPerMinute;
                var key = http.User.FindFirst("sub")?.Value ?? http.Connection.RemoteIpAddress?.ToString() ?? "unknown";
                return RateLimitPartition.GetFixedWindowLimiter(
                    key,
                    _ => new FixedWindowRateLimiterOptions { PermitLimit = limit, Window = TimeSpan.FromMinutes(1), QueueLimit = 0 });
            });
            options.AddPolicy(Endpoints.Endpoints.SyncRateLimit, http =>
            {
                var limit = http.RequestServices.GetRequiredService<IOptions<SecurityOptions>>().Value.SyncRequestsPerMinute;
                var key = http.User.FindFirst("sub")?.Value ?? http.Connection.RemoteIpAddress?.ToString() ?? "unknown";
                return RateLimitPartition.GetFixedWindowLimiter(
                    key,
                    _ => new FixedWindowRateLimiterOptions { PermitLimit = limit, Window = TimeSpan.FromMinutes(1), QueueLimit = 0 });
            });
            options.OnRejected = async (context, cancellationToken) =>
            {
                context.HttpContext.RequestServices.GetRequiredService<JarvisMetrics>().RateLimited(context.HttpContext.Request.Path);
                if (context.Lease.TryGetMetadata(MetadataName.RetryAfter, out var retryAfter))
                {
                    context.HttpContext.Response.Headers.RetryAfter = ((int)Math.Ceiling(retryAfter.TotalSeconds)).ToString(System.Globalization.CultureInfo.InvariantCulture);
                }

                await Problems.WriteAsync(context.HttpContext, ErrorCodes.RateLimited, "Too many requests.");
            };
        });

    private static void ConfigureForwardedHeaders(IServiceCollection services, IConfiguration configuration) =>
        services.Configure<ForwardedHeadersOptions>(options =>
        {
            options.ForwardedHeaders = ForwardedHeaders.XForwardedFor | ForwardedHeaders.XForwardedProto;
            options.KnownIPNetworks.Clear();
            options.KnownProxies.Clear();
            foreach (var network in configuration.GetSection("Jarvis:TrustedProxyNetworks").Get<string[]>() ?? [])
            {
                options.KnownIPNetworks.Add(System.Net.IPNetwork.Parse(network));
            }

            options.KnownProxies.Add(IPAddress.Loopback);
        });
}

/// <summary>Excepciones no controladas → 500 genérico; el detalle solo en logs.</summary>
public sealed class UnhandledExceptionHandler(ILogger<UnhandledExceptionHandler> logger) : IExceptionHandler
{
    public async ValueTask<bool> TryHandleAsync(HttpContext httpContext, Exception exception, CancellationToken cancellationToken)
    {
        if (exception is BadHttpRequestException badRequest)
        {
            var code = badRequest.StatusCode == StatusCodes.Status413PayloadTooLarge ? ErrorCodes.PayloadTooLarge : ErrorCodes.ValidationFailed;
            await Problems.WriteAsync(httpContext, code, code == ErrorCodes.PayloadTooLarge ? "Request body too large." : "Invalid request.");
            return true;
        }

        logger.LogError(exception, "Unhandled exception");
        await Problems.WriteAsync(httpContext, ErrorCodes.InternalError, "An unexpected error occurred.");
        return true;
    }
}
