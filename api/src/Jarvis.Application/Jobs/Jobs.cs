using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Domain.Jobs;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;

namespace Jarvis.Application.Jobs;

// Jobs programados (ADR-016): catálogo cerrado de tipos, API de administración y planificador/ejecutor del worker.

public sealed record JobTypeInfo(string Type, string Mode, bool RequiresApproval, string Description, Func<JsonElement, string?> ValidateParameters);

public static class JobCatalog
{
    public const string ReportSummary = "report.summary";
    public const string BackupVerify = "backup.verify";
    public const string HttpCheck = "http.check";
    public const string MemoryConsolidate = "memory.consolidate";
    public const string SyncCompact = "sync.compact";
    public const string MemoryPurgeExpired = "memory.purge_expired";

    public static readonly IReadOnlyDictionary<string, JobTypeInfo> Types = new Dictionary<string, JobTypeInfo>(StringComparer.Ordinal)
    {
        [ReportSummary] = new(ReportSummary, JobModes.ReadOnly, false, "Resumen de actividad (recuerdos, sincronizaciones, conflictos, jobs)", p => IntRange(p, "hours", 1, 168)),
        [BackupVerify] = new(BackupVerify, JobModes.ReadOnly, false, "Verifica el último backup (existencia, edad, tamaño y checksum)", p => IntRange(p, "maxAgeHours", 1, 720)),
        [HttpCheck] = new(HttpCheck, JobModes.ReadOnly, false, "Comprueba la disponibilidad de una URL de la allowlist del worker", p => RequiredString(p, "target", 64)),
        [MemoryConsolidate] = new(MemoryConsolidate, JobModes.Mutating, false, "Detecta duplicados (relaciones sugeridas, sin borrar) y propone habilidades desde experiencias verificadas", p => IntRange(p, "minSupport", 2, 20)),
        [SyncCompact] = new(SyncCompact, JobModes.Mutating, false, "Elimina recibos de idempotencia antiguos", p => IntRange(p, "receiptDays", 1, 365)),
        [MemoryPurgeExpired] = new(MemoryPurgeExpired, JobModes.Mutating, true, "Aplica la retención: convierte en tombstone los recuerdos vencidos", _ => null),
    };

    private static string? IntRange(JsonElement p, string name, int min, int max)
    {
        if (!p.TryGetProperty(name, out var value))
        {
            return null;
        }

        return value.ValueKind == JsonValueKind.Number && value.TryGetInt32(out var n) && n >= min && n <= max
            ? null
            : $"{name} must be an integer between {min} and {max}.";
    }

    private static string? RequiredString(JsonElement p, string name, int max) =>
        p.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String && value.GetString() is { Length: > 0 } s && s.Length <= max
            ? null
            : $"{name} is required (1-{max} characters).";
}

public sealed record JobDto(
    Guid Id,
    string Type,
    string Name,
    string ScheduleKind,
    int? IntervalSeconds,
    int? DailyMinuteUtc,
    DateTimeOffset? NextRunAt,
    bool Enabled,
    string Mode,
    string ApprovalState,
    JsonElement Parameters,
    int TimeoutSeconds,
    int MaxAttempts,
    DateTimeOffset CreatedAt);

public sealed record JobRunDto(
    Guid Id,
    Guid JobId,
    int Attempt,
    string Status,
    string TriggeredBy,
    DateTimeOffset ScheduledFor,
    DateTimeOffset? StartedAt,
    DateTimeOffset? FinishedAt,
    JsonElement? Result,
    JsonElement? Evidence,
    string? Error);

public sealed record CreateJobRequest(
    string? Type,
    string? Name,
    string? ScheduleKind,
    int? IntervalSeconds,
    int? DailyMinuteUtc,
    DateTimeOffset? RunAt,
    JsonElement? Parameters,
    int? TimeoutSeconds,
    int? MaxAttempts);

public sealed record UpdateJobRequest(string? Name, bool? Enabled, int? IntervalSeconds, int? DailyMinuteUtc, JsonElement? Parameters);

/// <summary>/api/v1/jobs: solo el owner. Crear, listar, aprobar, ejecutar ahora y consultar ejecuciones.</summary>
public sealed class JobAdminHandler(IJarvisDbContext db, TimeProvider clock)
{
    public async Task<Result<JobDto>> CreateAsync(Principal principal, CreateJobRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden("Only the owner manages jobs.");
        }

        if (request?.Type is not { } type || !JobCatalog.Types.TryGetValue(type, out var info))
        {
            return new AppError(ErrorCodes.JobTypeUnknown, $"Known job types: {string.Join(", ", JobCatalog.Types.Keys)}.");
        }

        var parameters = request.Parameters is { ValueKind: JsonValueKind.Object } p ? p : JarvisJson.EmptyObject;
        if (info.ValidateParameters(parameters) is { } invalid)
        {
            return AppError.Validation("parameters", invalid);
        }

        var schedule = request.ScheduleKind ?? ScheduleKinds.Once;
        if (!ScheduleKinds.All.Contains(schedule))
        {
            return AppError.Validation("scheduleKind", "scheduleKind must be once, interval or daily.");
        }

        if (schedule == ScheduleKinds.Interval && request.IntervalSeconds is not >= 60)
        {
            return AppError.Validation("intervalSeconds", "intervalSeconds must be >= 60.");
        }

        if (schedule == ScheduleKinds.Daily && request.DailyMinuteUtc is not (>= 0 and <= 1439))
        {
            return AppError.Validation("dailyMinuteUtc", "dailyMinuteUtc must be 0-1439.");
        }

        var name = string.IsNullOrWhiteSpace(request.Name) ? type : request.Name.Trim();
        if (name.Length > 100)
        {
            return AppError.Validation("name", "name must have at most 100 characters.");
        }

        var now = clock.GetUtcNow();
        var job = new Job
        {
            Id = Guid.CreateVersion7(now),
            OwnerId = principal.OwnerId,
            Type = type,
            Name = name,
            ScheduleKind = schedule,
            IntervalSeconds = schedule == ScheduleKinds.Interval ? request.IntervalSeconds : null,
            DailyMinuteUtc = schedule == ScheduleKinds.Daily ? request.DailyMinuteUtc : null,
            Mode = info.Mode,
            ApprovalState = info.RequiresApproval ? ApprovalStates.Pending : ApprovalStates.NotRequired,
            Parameters = parameters.GetRawText(),
            TimeoutSeconds = Math.Clamp(request.TimeoutSeconds ?? 300, 1, 3600),
            MaxAttempts = Math.Clamp(request.MaxAttempts ?? 3, 1, 10),
            CreatedById = principal.Id,
            CreatedAt = now,
            UpdatedAt = now,
        };
        job.NextRunAt = schedule switch
        {
            ScheduleKinds.Daily => Job.NextDaily(now, job.DailyMinuteUtc!.Value),
            _ => request.RunAt is { } at && at > now ? at : now,
        };
        db.Jobs.Add(job);
        db.AddEvent(new DomainEvent(
            EventTypes.JobCreated,
            principal.OwnerId,
            now,
            Data: new Dictionary<string, object?> { ["jobId"] = job.Id, ["type"] = type, ["approvalState"] = job.ApprovalState }));
        await db.SaveChangesAsync(cancellationToken);
        return ToDto(job);
    }

    public async Task<Result<IReadOnlyList<JobDto>>> ListAsync(Principal principal, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var jobs = await db.Jobs.AsNoTracking().OrderBy(j => j.CreatedAt).ToListAsync(cancellationToken);
        return jobs.Select(ToDto).ToList();
    }

    public async Task<Result<JobDto>> GetAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var job = await db.Jobs.AsNoTracking().FirstOrDefaultAsync(j => j.Id == id, cancellationToken);
        return job is null ? AppError.NotFound("Job") : ToDto(job);
    }

    public async Task<Result<JobDto>> UpdateAsync(Principal principal, Guid id, UpdateJobRequest? request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var job = await db.Jobs.FirstOrDefaultAsync(j => j.Id == id, cancellationToken);
        if (job is null)
        {
            return AppError.NotFound("Job");
        }

        var info = JobCatalog.Types[job.Type];
        var now = clock.GetUtcNow();
        if (request?.Name is { } name)
        {
            if (name.Trim().Length is 0 or > 100)
            {
                return AppError.Validation("name", "name must have 1-100 characters.");
            }

            job.Name = name.Trim();
        }

        if (request?.Parameters is { ValueKind: JsonValueKind.Object } parameters && parameters.GetRawText() != job.Parameters)
        {
            if (info.ValidateParameters(parameters) is { } invalid)
            {
                return AppError.Validation("parameters", invalid);
            }

            job.Parameters = parameters.GetRawText();
            if (info.RequiresApproval)
            {
                // Parámetros nuevos = otra acción: vuelve a necesitar aprobación.
                job.ApprovalState = ApprovalStates.Pending;
                job.ApprovedAt = null;
                job.ApprovedById = null;
            }
        }

        if (request?.IntervalSeconds is { } interval && job.ScheduleKind == ScheduleKinds.Interval)
        {
            if (interval < 60)
            {
                return AppError.Validation("intervalSeconds", "intervalSeconds must be >= 60.");
            }

            job.IntervalSeconds = interval;
        }

        if (request?.DailyMinuteUtc is { } minute && job.ScheduleKind == ScheduleKinds.Daily)
        {
            if (minute is < 0 or > 1439)
            {
                return AppError.Validation("dailyMinuteUtc", "dailyMinuteUtc must be 0-1439.");
            }

            job.DailyMinuteUtc = minute;
            job.NextRunAt = Job.NextDaily(now, minute);
        }

        if (request?.Enabled is { } enabled)
        {
            job.Enabled = enabled;
            if (enabled && job.NextRunAt is null && job.ScheduleKind != ScheduleKinds.Once)
            {
                job.NextRunAt = job.NextAfter(now);
            }
        }

        job.UpdatedAt = now;
        await db.SaveChangesAsync(cancellationToken);
        return ToDto(job);
    }

    public async Task<Result<JobDto>> ApproveAsync(Principal principal, Guid id, bool approve, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden("Only the owner approves jobs.");
        }

        var job = await db.Jobs.FirstOrDefaultAsync(j => j.Id == id, cancellationToken);
        if (job is null)
        {
            return AppError.NotFound("Job");
        }

        if (job.ApprovalState == ApprovalStates.NotRequired)
        {
            return ToDto(job);
        }

        var now = clock.GetUtcNow();
        job.ApprovalState = approve ? ApprovalStates.Approved : ApprovalStates.Rejected;
        job.ApprovedAt = approve ? now : null;
        job.ApprovedById = approve ? principal.Id : null;
        job.UpdatedAt = now;
        db.AddEvent(new DomainEvent(
            EventTypes.JobApproved,
            principal.OwnerId,
            now,
            Data: new Dictionary<string, object?> { ["jobId"] = job.Id, ["approved"] = approve }));
        await db.SaveChangesAsync(cancellationToken);
        return ToDto(job);
    }

    public async Task<Result<JobRunDto>> RunNowAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        var job = await db.Jobs.AsNoTracking().FirstOrDefaultAsync(j => j.Id == id, cancellationToken);
        if (job is null)
        {
            return AppError.NotFound("Job");
        }

        if (job.ApprovalState is ApprovalStates.Pending or ApprovalStates.Rejected)
        {
            return new AppError(ErrorCodes.JobNotRunnable, "The job needs the owner's approval first.");
        }

        if (await db.JobRuns.AnyAsync(r => r.JobId == id && (r.Status == RunStatuses.Queued || r.Status == RunStatuses.Running), cancellationToken))
        {
            return new AppError(ErrorCodes.JobNotRunnable, "The job already has an active run.");
        }

        var now = clock.GetUtcNow();
        var run = new JobRun
        {
            Id = Guid.CreateVersion7(now),
            JobId = job.Id,
            OwnerId = job.OwnerId,
            TriggeredBy = "manual",
            ScheduledFor = now,
            CreatedAt = now,
            UpdatedAt = now,
        };
        db.JobRuns.Add(run);
        try
        {
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (DbUpdateException exception) when (db.IsUniqueViolation(exception))
        {
            db.ResetTracking();
            return new AppError(ErrorCodes.JobNotRunnable, "The job already has an active run.");
        }

        return ToDto(run);
    }

    public async Task<Result<IReadOnlyList<JobRunDto>>> RunsAsync(Principal principal, Guid id, int? limit, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        if (!await db.Jobs.AnyAsync(j => j.Id == id, cancellationToken))
        {
            return AppError.NotFound("Job");
        }

        var runs = await db.JobRuns.AsNoTracking().Where(r => r.JobId == id).OrderByDescending(r => r.CreatedAt).Take(Math.Clamp(limit ?? 20, 1, 100)).ToListAsync(cancellationToken);
        return runs.Select(ToDto).ToList();
    }

    public static JobDto ToDto(Job j) => new(
        j.Id, j.Type, j.Name, j.ScheduleKind, j.IntervalSeconds, j.DailyMinuteUtc, j.NextRunAt, j.Enabled, j.Mode, j.ApprovalState,
        JarvisJson.ParseElement(j.Parameters), j.TimeoutSeconds, j.MaxAttempts, j.CreatedAt);

    public static JobRunDto ToDto(JobRun r) => new(
        r.Id, r.JobId, r.Attempt, r.Status, r.TriggeredBy, r.ScheduledFor, r.StartedAt, r.FinishedAt,
        r.Result is null ? null : JarvisJson.ParseElement(r.Result), r.Evidence is null ? null : JarvisJson.ParseElement(r.Evidence), r.Error);
}

// ---- Worker ---------------------------------------------------------------------------------

public sealed record JobContext(Job Job, JobRun Run, JsonElement Parameters, IServiceProvider Services);

public sealed record JobOutcome(object Result, object? Evidence = null);

/// <summary>Implementación de un tipo de job del catálogo. Se registra en el worker; nunca ejecuta código externo.</summary>
public interface IJobHandler
{
    string Type { get; }

    Task<JobOutcome> ExecuteAsync(JobContext context, CancellationToken cancellationToken);
}

public sealed class JobSchedulerOptions
{
    public string WorkerId { get; set; } = $"{Environment.MachineName}:{Environment.ProcessId}";

    public int BatchSize { get; set; } = 5;

    /// <summary>Margen de la lease sobre el timeout del job antes de que otra instancia lo reclame.</summary>
    public int LeaseGraceSeconds { get; set; } = 30;
}

/// <summary>
/// Planificador y ejecutor (ADR-016). <see cref="ScheduleDueAsync"/> materializa ejecuciones de jobs vencidos;
/// <see cref="ClaimAsync"/> reclama ejecuciones encoladas o con lease vencida; <see cref="ExecuteAsync"/> ejecuta una
/// con timeout y registra el resultado. Todo con FOR UPDATE SKIP LOCKED: varias instancias pueden convivir.
/// </summary>
public sealed class JobScheduler(
    IJarvisDbContext db,
    IMemorySql sql,
    IRequestContext context,
    IEnumerable<IJobHandler> handlers,
    IServiceProvider services,
    JobSchedulerOptions options,
    TimeProvider clock,
    ILogger<JobScheduler> logger)
{
    private readonly Dictionary<string, IJobHandler> _handlers = handlers.ToDictionary(h => h.Type, StringComparer.Ordinal);

    public async Task<int> ScheduleDueAsync(CancellationToken cancellationToken)
    {
        context.EnterSystemScope();
        return await db.InTransactionAsync(
            async ct =>
            {
                var now = clock.GetUtcNow();
                var due = await sql.ClaimDueJobsAsync(now, 50, ct);
                foreach (var job in due)
                {
                    db.JobRuns.Add(new JobRun
                    {
                        Id = Guid.CreateVersion7(now),
                        JobId = job.Id,
                        OwnerId = job.OwnerId,
                        ScheduledFor = job.NextRunAt ?? now,
                        CreatedAt = now,
                        UpdatedAt = now,
                    });
                    job.NextRunAt = job.NextAfter(job.NextRunAt is { } previous && previous > now.AddDays(-1) ? Max(previous, now) : now);
                    job.UpdatedAt = now;
                }

                await db.SaveChangesAsync(ct);
                return due.Count;
            },
            cancellationToken);
    }

    public async Task<IReadOnlyList<Guid>> ClaimAsync(CancellationToken cancellationToken)
    {
        context.EnterSystemScope();
        return await db.InTransactionAsync<IReadOnlyList<Guid>>(
            async ct =>
            {
                var now = clock.GetUtcNow();
                var runs = await sql.ClaimRunsAsync(now, options.BatchSize, ct);
                var claimed = new List<Guid>();
                foreach (var run in runs)
                {
                    var job = await db.Jobs.FirstAsync(j => j.Id == run.JobId, ct);
                    if (run.Status == RunStatuses.Running)
                    {
                        // Lease vencida: el worker anterior murió o se colgó.
                        if (run.Attempt >= job.MaxAttempts)
                        {
                            Finish(run, job, RunStatuses.Failed, now, error: "Lease expired on the last attempt (worker lost).");
                            continue;
                        }

                        run.Attempt += 1;
                    }

                    if (!job.CanRun && run.TriggeredBy == "schedule")
                    {
                        Finish(run, job, RunStatuses.Cancelled, now, error: "The job is disabled or not approved.");
                        continue;
                    }

                    run.Status = RunStatuses.Running;
                    run.WorkerId = options.WorkerId;
                    run.StartedAt = now;
                    run.LeaseUntil = now.AddSeconds(job.TimeoutSeconds + options.LeaseGraceSeconds);
                    run.UpdatedAt = now;
                    db.AddEvent(new DomainEvent(
                        EventTypes.JobRunStarted,
                        job.OwnerId,
                        now,
                        Data: new Dictionary<string, object?> { ["jobId"] = job.Id, ["runId"] = run.Id, ["type"] = job.Type, ["attempt"] = run.Attempt },
                        Actor: Actor.System));
                    claimed.Add(run.Id);
                }

                await db.SaveChangesAsync(ct);
                return claimed;
            },
            cancellationToken);
    }

    public async Task ExecuteAsync(Guid runId, CancellationToken cancellationToken)
    {
        context.EnterSystemScope();
        var run = await db.JobRuns.FirstAsync(r => r.Id == runId, cancellationToken);
        var job = await db.Jobs.FirstAsync(j => j.Id == run.JobId, cancellationToken);
        if (!_handlers.TryGetValue(job.Type, out var handler))
        {
            Finish(run, job, RunStatuses.Failed, clock.GetUtcNow(), error: $"No handler registered for '{job.Type}'.");
            await db.SaveChangesAsync(cancellationToken);
            return;
        }

        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(job.TimeoutSeconds));
        string status;
        JobOutcome? outcome = null;
        string? error = null;
        try
        {
            outcome = await handler.ExecuteAsync(new JobContext(job, run, JarvisJson.ParseElement(job.Parameters), services), timeout.Token);
            status = RunStatuses.Succeeded;
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            status = RunStatuses.TimedOut;
            error = $"Timed out after {job.TimeoutSeconds} s.";
        }
        catch (Exception exception) when (exception is not OperationCanceledException)
        {
            logger.LogWarning(exception, "Job {JobId} ({Type}) failed", job.Id, job.Type);
            status = RunStatuses.Failed;
            error = exception.Message.Length > 500 ? exception.Message[..500] : exception.Message;
        }

        // El handler pudo dejar cambios a medias si falló: se descartan.
        db.ResetTracking();
        run = await db.JobRuns.FirstAsync(r => r.Id == runId, CancellationToken.None);
        job = await db.Jobs.FirstAsync(j => j.Id == run.JobId, CancellationToken.None);
        var now = clock.GetUtcNow();
        if (status != RunStatuses.Succeeded && run.Attempt < job.MaxAttempts)
        {
            // Reintento con backoff: vuelve a la cola.
            run.Status = RunStatuses.Queued;
            run.Attempt += 1;
            run.ScheduledFor = now.AddSeconds(Math.Min(300, 15 * Math.Pow(2, run.Attempt - 1)));
            run.LeaseUntil = null;
            run.Error = error;
            run.UpdatedAt = now;
        }
        else
        {
            Finish(run, job, status, now, outcome, error);
        }

        await db.SaveChangesAsync(CancellationToken.None);
    }

    private void Finish(JobRun run, Job job, string status, DateTimeOffset now, JobOutcome? outcome = null, string? error = null)
    {
        run.Status = status;
        run.FinishedAt = now;
        run.LeaseUntil = null;
        run.UpdatedAt = now;
        run.Error = error;
        if (outcome is not null)
        {
            run.Result = Bounded(JsonSerializer.Serialize(outcome.Result, JarvisJson.Options));
            run.Evidence = outcome.Evidence is null ? null : Bounded(JsonSerializer.Serialize(outcome.Evidence, JarvisJson.Options));
        }

        if (job.ScheduleKind == ScheduleKinds.Once && run.TriggeredBy == "schedule")
        {
            job.Enabled = status != RunStatuses.Succeeded && job.Enabled;
        }

        db.AddEvent(new DomainEvent(
            status == RunStatuses.Succeeded ? EventTypes.JobRunSucceeded : EventTypes.JobRunFailed,
            job.OwnerId,
            now,
            Data: new Dictionary<string, object?> { ["jobId"] = job.Id, ["runId"] = run.Id, ["type"] = job.Type, ["status"] = status, ["attempt"] = run.Attempt },
            Actor: Actor.System));
    }

    private static string Bounded(string json) => json.Length <= 30_000 ? json : JsonSerializer.Serialize(new { truncated = true, size = json.Length });

    private static DateTimeOffset Max(DateTimeOffset a, DateTimeOffset b) => a > b ? a : b;
}
