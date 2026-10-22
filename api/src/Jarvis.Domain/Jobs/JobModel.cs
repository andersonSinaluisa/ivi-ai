namespace Jarvis.Domain.Jobs;

// Jobs programados que ejecuta Jarvis.Worker (ADR-016). Solo tipos registrados en código; nunca código generado.

public static class ScheduleKinds
{
    public const string Once = "once";
    public const string Interval = "interval";
    public const string Daily = "daily";

    public static readonly IReadOnlyList<string> All = [Once, Interval, Daily];
}

public static class JobModes
{
    public const string ReadOnly = "read_only";
    public const string Mutating = "mutating";
}

public static class ApprovalStates
{
    public const string NotRequired = "not_required";
    public const string Pending = "pending";
    public const string Approved = "approved";
    public const string Rejected = "rejected";
}

public static class RunStatuses
{
    public const string Queued = "queued";
    public const string Running = "running";
    public const string Succeeded = "succeeded";
    public const string Failed = "failed";
    public const string TimedOut = "timed_out";
    public const string Cancelled = "cancelled";

    public static bool IsActive(string status) => status is Queued or Running;
}

public sealed class Job
{
    public Guid Id { get; set; }

    public Guid OwnerId { get; set; }

    public string Type { get; set; } = string.Empty;

    public string Name { get; set; } = string.Empty;

    public string ScheduleKind { get; set; } = ScheduleKinds.Once;

    public int? IntervalSeconds { get; set; }

    /// <summary>Minuto del día (UTC) para la programación diaria.</summary>
    public int? DailyMinuteUtc { get; set; }

    public DateTimeOffset? NextRunAt { get; set; }

    public bool Enabled { get; set; } = true;

    public string Mode { get; set; } = JobModes.ReadOnly;

    public string ApprovalState { get; set; } = ApprovalStates.NotRequired;

    /// <summary>jsonb validado por el handler del tipo.</summary>
    public string Parameters { get; set; } = "{}";

    public int TimeoutSeconds { get; set; } = 300;

    public int MaxAttempts { get; set; } = 3;

    public Guid CreatedById { get; set; }

    public DateTimeOffset? ApprovedAt { get; set; }

    public Guid? ApprovedById { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }

    public bool CanRun => Enabled && ApprovalState is ApprovalStates.NotRequired or ApprovalStates.Approved;

    /// <summary>Próxima ejecución después de <paramref name="after"/>; null si no se repite.</summary>
    public DateTimeOffset? NextAfter(DateTimeOffset after) => ScheduleKind switch
    {
        ScheduleKinds.Interval when IntervalSeconds is > 0 => after.AddSeconds(IntervalSeconds.Value),
        ScheduleKinds.Daily when DailyMinuteUtc is { } minute => NextDaily(after, minute),
        _ => null,
    };

    public static DateTimeOffset NextDaily(DateTimeOffset after, int minuteUtc)
    {
        var day = new DateTimeOffset(after.UtcDateTime.Date, TimeSpan.Zero).AddMinutes(minuteUtc);
        return day > after ? day : day.AddDays(1);
    }
}

public sealed class JobRun
{
    public Guid Id { get; set; }

    public Guid JobId { get; set; }

    public Guid OwnerId { get; set; }

    public int Attempt { get; set; } = 1;

    public string Status { get; set; } = RunStatuses.Queued;

    public string TriggeredBy { get; set; } = "schedule";

    public DateTimeOffset ScheduledFor { get; set; }

    public DateTimeOffset? LeaseUntil { get; set; }

    public string? WorkerId { get; set; }

    public DateTimeOffset? StartedAt { get; set; }

    public DateTimeOffset? FinishedAt { get; set; }

    public string? Result { get; set; }

    public string? Evidence { get; set; }

    public string? Error { get; set; }

    public DateTimeOffset CreatedAt { get; set; }

    public DateTimeOffset UpdatedAt { get; set; }
}
