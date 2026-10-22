using Jarvis.Api.Http;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Jobs;
using Jarvis.Application.Sync;
using Jarvis.Contracts.Sync;
using Microsoft.AspNetCore.Mvc;

namespace Jarvis.Api.Endpoints;

/// <summary>
/// Memoria, sincronización, habilidades y jobs (docs/03-protocols/sync-protocol.md). Ningún endpoint ejecuta código ni
/// habilidades: el Cloud guarda, versiona y transporta.
/// </summary>
public static class MemoryEndpoints
{
    public static void MapJarvisMemoryApi(this IEndpointRouteBuilder app)
    {
        var api = app.MapGroup("/api/v1");

        api.MapGet("/health", async (IMemorySql sql, TimeProvider clock, CancellationToken ct) =>
        {
            var vector = await sql.VectorExtensionAvailableAsync(ct) ? "pgvector" : "application";
            return Results.Ok(new HealthResponse("ok", "v1", clock.GetUtcNow(), new HealthFeatures(SyncProtocol.Version, vector)));
        }).AllowAnonymous();

        var sync = api.MapGroup("/sync").RequireRateLimiting(Endpoints.SyncRateLimit);
        sync.MapPost("/push", async ([FromBody] PushRequest? request, HttpContext http, SyncPushHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(http.Principal(), request, ct)).ToOk(http))
            .RequireAuthorization(Policies.DeviceOnly);
        sync.MapPost("/pull", async ([FromBody] PullRequest? request, HttpContext http, SyncPullHandler handler, CancellationToken ct) =>
            (await handler.HandleAsync(http.Principal(), request, ct)).ToOk(http))
            .RequireAuthorization(Policies.DeviceOnly);
        sync.MapGet("/status", async (HttpContext http, SyncDeviceHandler handler, CancellationToken ct) =>
            (await handler.StatusAsync(http.Principal(), ct)).ToOk(http))
            .RequireAuthorization(Policies.UserOrDevice);
        sync.MapGet("/devices", async (HttpContext http, SyncDeviceHandler handler, CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), ct)).ToHttp(http, items => Results.Ok(new { items })))
            .RequireAuthorization(Policies.UserOnly);
        sync.MapPut("/devices/{deviceId:guid}", async (Guid deviceId, [FromBody] SetSyncScopesRequest? request, HttpContext http, SyncDeviceHandler handler, CancellationToken ct) =>
            (await handler.SetScopesAsync(http.Principal(), deviceId, request, ct)).ToOk(http))
            .RequireAuthorization(Policies.UserOnly);
        sync.MapGet("/conflicts", async (HttpContext http, SyncConflictHandler handler, [FromQuery] string? status, CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), status, ct)).ToHttp(http, items => Results.Ok(new { items })))
            .RequireAuthorization(Policies.UserOrDevice);
        sync.MapPost("/conflicts/{id:guid}/resolve", async (Guid id, [FromBody] ResolveConflictRequest? request, HttpContext http, SyncConflictHandler handler, CancellationToken ct) =>
            (await handler.ResolveAsync(http.Principal(), id, request, ct)).ToOk(http))
            .RequireAuthorization(Policies.UserOrDevice);

        var memories = api.MapGroup("/memories").RequireAuthorization(Policies.UserOrDevice).RequireRateLimiting(Endpoints.SyncRateLimit);
        memories.MapGet("/", async (
            HttpContext http,
            MemoryQueryHandler handler,
            [FromQuery] string? kind,
            [FromQuery] string? q,
            [FromQuery] int? limit,
            [FromQuery] string? cursor,
            CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), kind, q, limit, cursor, ct)).ToOk(http));
        memories.MapGet("/{id:guid}", async (Guid id, HttpContext http, MemoryQueryHandler handler, CancellationToken ct) =>
            (await handler.GetAsync(http.Principal(), id, ct)).ToOk(http));
        memories.MapGet("/{id:guid}/versions", async (Guid id, HttpContext http, MemoryQueryHandler handler, CancellationToken ct) =>
            (await handler.VersionsAsync(http.Principal(), id, ct)).ToHttp(http, items => Results.Ok(new { items })));
        memories.MapPost("/search", async ([FromBody] MemorySearchRequest? request, HttpContext http, MemoryQueryHandler handler, CancellationToken ct) =>
            (await handler.SearchAsync(http.Principal(), request, ct)).ToOk(http));
        memories.MapGet("/export", async (HttpContext http, MemoryQueryHandler handler, CancellationToken ct) =>
            (await handler.ExportAsync(http.Principal(), ct)).ToHttp(http, export =>
            {
                http.Response.Headers.CacheControl = "no-store";
                return Results.Ok(export);
            }));

        var experiences = api.MapGroup("/experiences").RequireAuthorization(Policies.UserOrDevice).RequireRateLimiting(Endpoints.SyncRateLimit);
        experiences.MapGet("/", async (HttpContext http, ExperienceQueryHandler handler, [FromQuery] string? signature, [FromQuery] int? limit, CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), signature, limit, ct)).ToHttp(http, items => Results.Ok(new { items })));
        experiences.MapGet("/similar", async (HttpContext http, ExperienceQueryHandler handler, [FromQuery] string? goal, [FromQuery] int? limit, CancellationToken ct) =>
            (await handler.SimilarAsync(http.Principal(), goal, limit, ct)).ToHttp(http, items => Results.Ok(new { items })));

        var skills = api.MapGroup("/skills").RequireAuthorization(Policies.UserOrDevice);
        skills.MapGet("/", async (HttpContext http, SkillHandler handler, [FromQuery] string? status, CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), status, ct)).ToHttp(http, items => Results.Ok(new { items })));
        skills.MapGet("/proposals", async (HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.ProposalsAsync(http.Principal(), ct)).ToHttp(http, items => Results.Ok(new { items })));
        skills.MapPost("/proposals/{id:guid}/accept", async (Guid id, HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.DecideProposalAsync(http.Principal(), id, accept: true, ct)).ToOk(http));
        skills.MapPost("/proposals/{id:guid}/reject", async (Guid id, HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.DecideProposalAsync(http.Principal(), id, accept: false, ct)).ToOk(http));
        skills.MapGet("/{id:guid}", async (Guid id, HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.GetAsync(http.Principal(), id, ct)).ToOk(http));
        skills.MapGet("/{id:guid}/versions", async (Guid id, HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.VersionsAsync(http.Principal(), id, ct)).ToHttp(http, items => Results.Ok(new { items })));
        skills.MapPost("/", async ([FromBody] CreateSkillRequest? request, HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.CreateAsync(http.Principal(), request, ct)).ToHttp(http, skill => Results.Created($"/api/v1/skills/{skill.Id}", skill)));
        skills.MapPatch("/{id:guid}", async (Guid id, [FromBody] UpdateSkillStatusRequest? request, HttpContext http, SkillHandler handler, CancellationToken ct) =>
            (await handler.SetStatusAsync(http.Principal(), id, request, ct)).ToOk(http));

        var jobs = api.MapGroup("/jobs").RequireAuthorization(Policies.UserOnly);
        jobs.MapGet("/", async (HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.ListAsync(http.Principal(), ct)).ToHttp(http, items => Results.Ok(new { items, types = JobCatalog.Types.Values.Select(t => new { t.Type, t.Mode, t.RequiresApproval, t.Description }) })));
        jobs.MapPost("/", async ([FromBody] CreateJobRequest? request, HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.CreateAsync(http.Principal(), request, ct)).ToHttp(http, job => Results.Created($"/api/v1/jobs/{job.Id}", job)));
        jobs.MapGet("/{id:guid}", async (Guid id, HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.GetAsync(http.Principal(), id, ct)).ToOk(http));
        jobs.MapPatch("/{id:guid}", async (Guid id, [FromBody] UpdateJobRequest? request, HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.UpdateAsync(http.Principal(), id, request, ct)).ToOk(http));
        jobs.MapPost("/{id:guid}/approve", async (Guid id, HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.ApproveAsync(http.Principal(), id, approve: true, ct)).ToOk(http));
        jobs.MapPost("/{id:guid}/reject", async (Guid id, HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.ApproveAsync(http.Principal(), id, approve: false, ct)).ToOk(http));
        jobs.MapPost("/{id:guid}/run", async (Guid id, HttpContext http, JobAdminHandler handler, CancellationToken ct) =>
            (await handler.RunNowAsync(http.Principal(), id, ct)).ToHttp(http, run => Results.Accepted($"/api/v1/jobs/{id}/runs", run)));
        jobs.MapGet("/{id:guid}/runs", async (Guid id, HttpContext http, JobAdminHandler handler, [FromQuery] int? limit, CancellationToken ct) =>
            (await handler.RunsAsync(http.Principal(), id, limit, ct)).ToHttp(http, items => Results.Ok(new { items })));
    }
}
