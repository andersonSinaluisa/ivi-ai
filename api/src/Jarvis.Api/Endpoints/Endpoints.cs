using Jarvis.Api.Http;
using Jarvis.Application.Auth;
using Jarvis.Application.Devices;
using Jarvis.Application.Events;
using Jarvis.Application.Tasks;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;
using Jarvis.Contracts.Rest;
using Microsoft.AspNetCore.Mvc;

namespace Jarvis.Api.Endpoints;

/// <summary>REST API v1 (docs/03-protocols/rest-api.md). No existe ningún endpoint de ejecución genérica.</summary>
public static class Endpoints
{
    public const string AuthRateLimit = "auth";
    public const string TaskCreateRateLimit = "task-create";
    public const string SyncRateLimit = "sync";

    public static void MapJarvisApi(this IEndpointRouteBuilder app)
    {
        var api = app.MapGroup("/api/v1");
        MapAuth(api.MapGroup("/auth"));
        api.MapGet("/me", async (HttpContext http, MeHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(http.Principal(), ct)).ToOk(http))
            .RequireAuthorization(Policies.UserOrDevice);
        MapDevices(api.MapGroup("/devices").RequireAuthorization(Policies.UserOnly));
        MapTasks(api.MapGroup("/tasks").RequireAuthorization(Policies.UserOrDevice));
        MapEvents(api.MapGroup("/events").RequireAuthorization(Policies.UserOnly));
        app.MapJarvisMemoryApi();
    }

    private static void MapAuth(RouteGroupBuilder auth)
    {
        auth.MapPost("/login", async (LoginRequest request, HttpContext http, LoginHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(request.Email, request.Password, http.Connection.RemoteIpAddress?.ToString(), ct)).ToOk(http))
            .AllowAnonymous()
            .RequireRateLimiting(AuthRateLimit);

        auth.MapPost("/refresh", async (RefreshRequest request, HttpContext http, RefreshHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(request.RefreshToken, ct)).ToOk(http))
            .AllowAnonymous()
            .RequireRateLimiting(AuthRateLimit);

        auth.MapPost("/logout", async (LogoutRequest request, HttpContext http, LogoutHandler handler, CancellationToken ct) =>
        {
            await handler.HandleAsync(http.Principal().Id, request.RefreshToken, ct);
            return Results.NoContent();
        }).RequireAuthorization(Policies.UserOnly);

        auth.MapPost("/device-token", async (DeviceTokenRequest request, HttpContext http, DeviceTokenHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(request.DeviceId, request.Secret, ct)).ToOk(http))
            .AllowAnonymous()
            .RequireRateLimiting(AuthRateLimit);
    }

    private static void MapDevices(RouteGroupBuilder devices)
    {
        devices.MapPost("/", async (CreateDeviceRequest request, HttpContext http, RegisterDeviceHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(http.Principal(), request, ct)).ToHttp(http, response =>
            {
                http.Response.Headers.CacheControl = "no-store";
                return Results.Created($"/api/v1/devices/{response.Device.Id}", response);
            }));

        devices.MapGet("/", async (
            HttpContext http,
            DeviceQueryHandler handler,
            [FromQuery] string? type,
            [FromQuery] string? status,
            [FromQuery] int? limit,
            [FromQuery] string? cursor,
            CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), new ListDevicesQuery(type, status, limit, cursor), ct)).ToOk(http));

        devices.MapGet("/{id:guid}", async (Guid id, HttpContext http, DeviceQueryHandler handler, CancellationToken ct) =>
            (await handler.GetAsync(http.Principal(), id, ct)).ToOk(http));

        devices.MapPatch("/{id:guid}", async (Guid id, RenameDeviceRequest request, HttpContext http, DeviceAdminHandler handler, CancellationToken ct) =>
            (await handler.RenameAsync(http.Principal(), id, request.Name, ct)).ToOk(http));

        devices.MapPost("/{id:guid}/disable", async (Guid id, HttpContext http, DeviceAdminHandler handler, CancellationToken ct) =>
            (await handler.DisableAsync(http.Principal(), id, ct)).ToOk(http));

        devices.MapPost("/{id:guid}/enable", async (Guid id, HttpContext http, DeviceAdminHandler handler, CancellationToken ct) =>
            (await handler.EnableAsync(http.Principal(), id, ct)).ToOk(http));

        devices.MapPost("/{id:guid}/revoke", async (Guid id, [FromBody] RevokeDeviceRequest? request, HttpContext http, DeviceAdminHandler handler, CancellationToken ct) =>
            (await handler.RevokeAsync(http.Principal(), id, request?.Reason, ct)).ToOk(http));

        devices.MapPost("/{id:guid}/credentials/rotate", async (Guid id, HttpContext http, DeviceAdminHandler handler, CancellationToken ct) =>
            (await handler.RotateCredentialAsync(http.Principal(), id, ct)).ToHttp(http, credential =>
            {
                http.Response.Headers.CacheControl = "no-store";
                return Results.Ok(credential);
            }));

        devices.MapGet("/{id:guid}/grants", async (Guid id, HttpContext http, GrantHandler handler, CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), id, ct)).ToHttp(http, grants => Results.Ok(new { items = grants })));

        devices.MapPost("/{id:guid}/grants", async (Guid id, CreateGrantRequest request, HttpContext http, GrantHandler handler, CancellationToken ct) =>
            (await handler.CreateAsync(http.Principal(), id, request, ct)).ToHttp(http, grant =>
                Results.Created($"/api/v1/devices/{id}/grants/{grant.Id}", grant)));

        devices.MapDelete("/{id:guid}/grants/{grantId:guid}", async (Guid id, Guid grantId, HttpContext http, GrantHandler handler, CancellationToken ct) =>
            (await handler.DeleteAsync(http.Principal(), id, grantId, ct)).ToHttp(http, _ => Results.NoContent()));
    }

    private static void MapTasks(RouteGroupBuilder tasks)
    {
        tasks.MapPost("/", async (
            CreateTaskRequest request,
            HttpContext http,
            CreateTaskHandler handler,
            [FromHeader(Name = "Idempotency-Key")] string? idempotencyKey,
            CancellationToken ct) =>
        {
            var command = new CreateTaskCommand(
                request.TargetDeviceId,
                request.Type,
                request.Payload,
                request.ExpiresInSeconds,
                request.MaxDurationSeconds,
                request.CorrelationId,
                string.IsNullOrEmpty(idempotencyKey) ? null : idempotencyKey,
                "rest",
                request.TaskId,
                request.ExpiresAt,
                request.Signature is null
                    ? null
                    : new SignatureInfo(request.Signature.Alg ?? string.Empty, request.Signature.KeyId ?? string.Empty, request.Signature.Value ?? string.Empty));
            return (await handler.HandleAsync(http.Principal(), command, ct)).ToHttp(http, result =>
            {
                var dto = TaskMapper.ToDto(result.Task, includeDetails: true);
                if (result.Duplicate)
                {
                    http.Response.Headers["Idempotent-Replayed"] = "true";
                    return Results.Ok(dto);
                }

                return Results.Created($"/api/v1/tasks/{dto.Id}", dto);
            });
        }).RequireRateLimiting(TaskCreateRateLimit);

        tasks.MapGet("/", async (
            HttpContext http,
            TaskQueryHandler handler,
            [FromQuery(Name = "status")] string[]? status,
            [FromQuery] Guid? targetDeviceId,
            [FromQuery] Guid? sourceDeviceId,
            [FromQuery] string? type,
            [FromQuery] Guid? correlationId,
            [FromQuery] DateTimeOffset? createdFrom,
            [FromQuery] DateTimeOffset? createdTo,
            [FromQuery] int? limit,
            [FromQuery] string? cursor,
            CancellationToken ct) =>
            (await handler.ListAsync(
                http.Principal(),
                new TaskListQuery(status ?? [], targetDeviceId, sourceDeviceId, type, correlationId, createdFrom, createdTo, limit, cursor),
                ct)).ToOk(http));

        tasks.MapGet("/{id:guid}", async (Guid id, HttpContext http, TaskQueryHandler handler, CancellationToken ct) =>
            (await handler.GetAsync(http.Principal(), id, ct)).ToOk(http));

        // HU-057. 202 = "transmitida al device", nunca "aprobada": quien decide es el agente (ADR-012).
        tasks.MapPost("/{id:guid}/approval", async (
            Guid id,
            [FromBody] ApprovalDecisionRequest request,
            HttpContext http,
            DecideTaskApprovalHandler handler,
            CancellationToken ct) =>
        {
            var command = new ApprovalDecisionCommand(
                request.Decision,
                request.Nonce,
                request.DecidedAt,
                request.Signature is null
                    ? null
                    : new SignatureInfo(request.Signature.Alg ?? string.Empty, request.Signature.KeyId ?? string.Empty, request.Signature.Value ?? string.Empty));
            return (await handler.HandleAsync(http.Principal(), id, command, ct)).ToHttp(
                http,
                approval => Results.Accepted($"/api/v1/tasks/{id}", TaskMapper.ToDto(approval)));
        });

        tasks.MapPost("/{id:guid}/cancel", async (Guid id, [FromBody] CancelTaskRequest? request, HttpContext http, CancelTaskHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(http.Principal(), id, ct)).ToHttp(http, result =>
            {
                var dto = TaskMapper.ToDto(result.Task, includeDetails: true);
                return result.Immediate ? Results.Ok(dto) : Results.Accepted($"/api/v1/tasks/{id}", dto);
            }));
    }

    private static void MapEvents(RouteGroupBuilder events) =>
        events.MapGet("/", async (
            HttpContext http,
            EventQueryHandler handler,
            [FromQuery] long? after,
            [FromQuery(Name = "type")] string[]? type,
            [FromQuery] Guid? deviceId,
            [FromQuery] Guid? taskId,
            [FromQuery] Guid? correlationId,
            [FromQuery] DateTimeOffset? from,
            [FromQuery] DateTimeOffset? to,
            [FromQuery] int? limit,
            CancellationToken ct) =>
            (await handler.ListAsync(
                http.Principal(),
                new EventListQuery(after, type ?? [], deviceId, taskId, correlationId, from, to, limit),
                ct)).ToOk(http));

    /// <summary>Cuerpos JSON inválidos o ausentes → 400 VALIDATION_FAILED en Problem Details.</summary>
    public static IResult BadRequestBody(HttpContext http) =>
        Problems.Create(http, ErrorCodes.ValidationFailed, "Request body is missing or is not valid JSON.");
}
