using System.Diagnostics;
using System.Security.Claims;
using System.Text;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Infrastructure.Security;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Authorization.Policy;
using Microsoft.AspNetCore.Mvc;

namespace Jarvis.Api.Http;

public static class Policies
{
    public const string UserOnly = "UserOnly";
    public const string DeviceOnly = "DeviceOnly";
    public const string UserOrDevice = "UserOrDevice";
}

public static class PrincipalExtensions
{
    public static Principal? ToJarvisPrincipal(this ClaimsPrincipal user)
    {
        var type = user.FindFirst(JarvisClaims.Type)?.Value;
        if (!Guid.TryParse(user.FindFirst("sub")?.Value, out var id) || !Guid.TryParse(user.FindFirst(JarvisClaims.OwnerId)?.Value, out var ownerId))
        {
            return null;
        }

        return type switch
        {
            JarvisClaims.UserType => new Principal(PrincipalType.User, id, ownerId),
            JarvisClaims.DeviceType => new Principal(PrincipalType.Device, id, ownerId),
            _ => null,
        };
    }

    /// <summary>Principal del request (garantizado por las políticas de autorización).</summary>
    public static Principal Principal(this HttpContext context) =>
        context.RequestServices.GetRequiredService<IRequestContext>().Principal
        ?? throw new InvalidOperationException("Endpoint requires an authenticated principal.");
}

/// <summary>Traduce errores de aplicación a RFC 9457 Problem Details (docs/03-protocols/error-model.md).</summary>
public static class Problems
{
    private static readonly Dictionary<string, int> StatusByCode = new(StringComparer.Ordinal)
    {
        [ErrorCodes.ValidationFailed] = 400,
        [ErrorCodes.IdempotencyKeyRequired] = 400,
        [ErrorCodes.Unauthenticated] = 401,
        [ErrorCodes.InvalidCredentials] = 401,
        [ErrorCodes.InvalidRefreshToken] = 401,
        [ErrorCodes.Forbidden] = 403,
        [ErrorCodes.TaskNotGranted] = 403,
        [ErrorCodes.DeviceDisabled] = 403,
        [ErrorCodes.NotFound] = 404,
        [ErrorCodes.DeviceNameTaken] = 409,
        [ErrorCodes.DeviceRevoked] = 409,
        [ErrorCodes.GrantAlreadyExists] = 409,
        [ErrorCodes.TargetDeviceUnavailable] = 409,
        [ErrorCodes.InvalidStateTransition] = 409,
        [ErrorCodes.TaskIdInUse] = 409,
        [ErrorCodes.ApprovalAlreadyDecided] = 409,
        [ErrorCodes.NonceReused] = 409,
        [ErrorCodes.PayloadTooLarge] = 413,
        [ErrorCodes.IdempotencyKeyReused] = 422,
        [ErrorCodes.TaskTypeNotSupported] = 422,
        [ErrorCodes.SameDevice] = 422,
        [ErrorCodes.DeviceLimitReached] = 422,
        [ErrorCodes.AccountLocked] = 423,
        [ErrorCodes.RateLimited] = 429,
        [ErrorCodes.InternalError] = 500,
        [ErrorCodes.ServiceUnavailable] = 503,
        [ErrorCodes.SyncScopeMissing] = 403,
        [ErrorCodes.UnsupportedProtocol] = 400,
        [ErrorCodes.ContentRejected] = 422,
        [ErrorCodes.ReferenceNotFound] = 422,
        [ErrorCodes.StaleEmbedding] = 422,
        [ErrorCodes.ConflictAlreadyResolved] = 409,
        [ErrorCodes.JobTypeUnknown] = 422,
        [ErrorCodes.JobNotRunnable] = 409,
    };

    public static int StatusFor(string code) => StatusByCode.TryGetValue(code, out var status) ? status : 400;

    public static IResult From(HttpContext http, AppError error) => Create(http, error.Code, error.Message, error.Errors);

    public static IResult Create(HttpContext http, string code, string detail, IReadOnlyDictionary<string, string[]>? errors = null) =>
        Results.Problem(Build(http, code, detail, errors));

    public static ProblemDetails Build(HttpContext http, string code, string detail, IReadOnlyDictionary<string, string[]>? errors = null)
    {
        var problem = new ProblemDetails
        {
            Type = $"urn:jarvis:problem:{code.ToLowerInvariant().Replace('_', '-')}",
            Title = Title(code),
            Status = StatusFor(code),
            Detail = detail,
            Instance = http.Request.Path,
        };
        problem.Extensions["code"] = code;
        problem.Extensions["correlationId"] = http.RequestServices.GetService<IRequestContext>()?.CorrelationId;
        problem.Extensions["traceId"] = Activity.Current?.Id ?? http.TraceIdentifier;
        if (errors is not null)
        {
            problem.Extensions["errors"] = errors;
        }

        return problem;
    }

    public static Task WriteAsync(HttpContext http, string code, string detail)
    {
        var problem = Build(http, code, detail);
        http.Response.StatusCode = problem.Status!.Value;
        return http.Response.WriteAsJsonAsync(problem, (System.Text.Json.JsonSerializerOptions?)null, "application/problem+json");
    }

    private static string Title(string code)
    {
        var words = code.ToLowerInvariant().Split('_');
        var builder = new StringBuilder();
        foreach (var word in words)
        {
            builder.Append(builder.Length == 0 ? char.ToUpperInvariant(word[0]) + word[1..] : " " + word);
        }

        return builder.ToString();
    }
}

public static class ResultHttpExtensions
{
    public static IResult ToHttp<T>(this Result<T> result, HttpContext http, Func<T, IResult> onSuccess) =>
        result.IsSuccess ? onSuccess(result.Value) : Problems.From(http, result.Error!);

    public static IResult ToOk<T>(this Result<T> result, HttpContext http) => result.ToHttp(http, value => Results.Ok(value));
}

/// <summary>401/403 de autorización también en Problem Details.</summary>
public sealed class ProblemAuthorizationResultHandler : IAuthorizationMiddlewareResultHandler
{
    private readonly AuthorizationMiddlewareResultHandler _default = new();

    public async Task HandleAsync(RequestDelegate next, HttpContext context, AuthorizationPolicy policy, PolicyAuthorizationResult authorizeResult)
    {
        if (authorizeResult.Challenged)
        {
            context.Response.Headers.WWWAuthenticate = "Bearer";
            await Problems.WriteAsync(context, ErrorCodes.Unauthenticated, "Authentication is required.");
            return;
        }

        if (authorizeResult.Forbidden)
        {
            await Problems.WriteAsync(context, ErrorCodes.Forbidden, "Operation not allowed for this principal.");
            return;
        }

        await _default.HandleAsync(next, context, policy, authorizeResult);
    }
}

/// <summary>HU-004: correlation id por request (X-Correlation-Id, X-Request-Id de nginx o generado) y scope de logging.</summary>
public sealed class CorrelationMiddleware(RequestDelegate next, ILogger<CorrelationMiddleware> logger)
{
    public const string HeaderName = "X-Correlation-Id";

    public async Task InvokeAsync(HttpContext context, IRequestContext requestContext)
    {
        var correlationId = Guid.TryParse(context.Request.Headers[HeaderName], out var provided) ? provided : Guid.CreateVersion7();
        requestContext.CorrelationId = correlationId;
        context.Response.OnStarting(() =>
        {
            context.Response.Headers[HeaderName] = correlationId.ToString("D");
            return Task.CompletedTask;
        });

        using var scope = logger.BeginScope(new Dictionary<string, object?>
        {
            ["CorrelationId"] = correlationId,
            ["TraceId"] = Activity.Current?.TraceId.ToString() ?? context.TraceIdentifier,
            ["RequestId"] = context.Request.Headers["X-Request-Id"].ToString(),
        });
        await next(context);
    }
}

/// <summary>Publica el principal autenticado en el contexto de aplicación (filtro por owner del DbContext).</summary>
public sealed class PrincipalContextMiddleware(RequestDelegate next, ILogger<PrincipalContextMiddleware> logger)
{
    public async Task InvokeAsync(HttpContext context, IRequestContext requestContext)
    {
        var principal = context.User.Identity?.IsAuthenticated == true ? context.User.ToJarvisPrincipal() : null;
        requestContext.Principal = principal;
        if (principal is null)
        {
            await next(context);
            return;
        }

        using var scope = logger.BeginScope(new Dictionary<string, object?>
        {
            ["OwnerId"] = principal.OwnerId,
            ["PrincipalType"] = principal.Type.ToString(),
            ["PrincipalId"] = principal.Id,
        });
        await next(context);
    }
}

/// <summary>El puerto de métricas (8081) solo sirve /metrics; el resto de puertos no sirve /metrics.</summary>
public sealed class MetricsPortMiddleware(RequestDelegate next, IConfiguration configuration)
{
    private readonly int _metricsPort = configuration.GetValue("Jarvis:MetricsPort", 8081);

    public Task InvokeAsync(HttpContext context)
    {
        var isMetricsPort = context.Connection.LocalPort == _metricsPort;
        var isMetricsPath = context.Request.Path.StartsWithSegments("/metrics", StringComparison.Ordinal);
        if (isMetricsPort != isMetricsPath)
        {
            context.Response.StatusCode = StatusCodes.Status404NotFound;
            return Task.CompletedTask;
        }

        return next(context);
    }
}

/// <summary>
/// Rechaza cuerpos declarados mayores al límite de la ruta (64 KiB; 1 MiB en el push de sincronización) con Problem
/// Details, y fija el mismo límite en Kestrel para cuerpos sin Content-Length.
/// </summary>
public sealed class RequestSizeLimitMiddleware(RequestDelegate next)
{
    public Task InvokeAsync(HttpContext context)
    {
        var limit = JarvisApiHost.BodyLimitFor(context.Request.Path);
        if (context.Request.ContentLength > limit)
        {
            return Problems.WriteAsync(context, ErrorCodes.PayloadTooLarge, "Request body too large.");
        }

        if (context.Features.Get<Microsoft.AspNetCore.Http.Features.IHttpMaxRequestBodySizeFeature>() is { IsReadOnly: false } feature)
        {
            feature.MaxRequestBodySize = limit;
        }

        return next(context);
    }
}
