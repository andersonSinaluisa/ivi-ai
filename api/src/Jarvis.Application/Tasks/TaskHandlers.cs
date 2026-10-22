using System.Text.Json;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Contracts;
using Jarvis.Contracts.Mqtt;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Tasks;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Tasks;

public static class TaskMapper
{
    public static TaskApprovalDto ToDto(TaskApproval approval) =>
        new(
            approval.Id,
            approval.TaskId,
            approval.Decision.ToString().ToUpperInvariant(),
            new RequestedByDto(approval.DecidedByType.ToString(), approval.DecidedByUserId ?? approval.DecidedByDeviceId!.Value),
            approval.DecidedAt,
            approval.Nonce,
            new SignatureDto(approval.SignatureAlg, approval.SignatureKeyId, approval.SignatureValue),
            approval.CreatedAt);

    public static TaskDto ToDto(JarvisTask task, bool includeDetails) =>
        new(
            task.Id,
            task.Type,
            task.Status.ToString(),
            includeDetails ? JsonText.ToElement(task.Payload) : null,
            new RequestedByDto(task.RequestedByType.ToString(), task.RequestedByType == ActorType.Device ? task.SourceDeviceId!.Value : task.RequestedByUserId!.Value),
            task.SourceDeviceId,
            task.TargetDeviceId,
            task.CorrelationId,
            task.IdempotencyKey,
            task.AttemptCount,
            task.ExpiresAt,
            task.MaxDurationSeconds,
            task.CreatedAt,
            task.DispatchedAt,
            task.DeliveredAt,
            task.StartedAt,
            task.CompletedAt,
            task.CancelRequestedAt,
            task.ProgressPercent is null && task.ProgressMessage is null ? null : new TaskProgressDto(task.ProgressPercent, task.ProgressMessage),
            includeDetails ? JsonText.ToNullableElement(task.Result) : null,
            task.ErrorCode is null ? null : new TaskErrorDto(task.ErrorCode, task.ErrorMessage),
            task.Signature is null ? null : new SignatureDto(task.Signature.Alg, task.Signature.KeyId, task.Signature.Value));

    public static TaskUpdateData ToUpdate(JarvisTask task, DateTimeOffset now) =>
        new(
            task.Id,
            task.Type,
            task.TargetDeviceId,
            task.Status.ToString(),
            task.LastUpdateAt ?? task.CompletedAt ?? now,
            task.ProgressPercent is null && task.ProgressMessage is null ? null : new ProgressInfo(task.ProgressPercent, task.ProgressMessage),
            task.Status == JarvisTaskStatus.Completed ? JsonText.ToNullableElement(task.Result) : null,
            task.ErrorCode is null ? null : new ErrorInfo(task.ErrorCode, task.ErrorMessage));

    public static TaskDispatchData ToDispatch(JarvisTask task) =>
        new(
            task.Id,
            task.Type,
            JsonText.ToElement(task.Payload),
            task.AttemptCount,
            task.CreatedAt,
            task.ExpiresAt,
            task.MaxDurationSeconds,
            new RequestedByInfo(task.RequestedByType.ToString(), task.RequestedByType == ActorType.Device ? task.SourceDeviceId!.Value : task.RequestedByUserId!.Value),
            task.CancelRequestedAt is not null,
            task.Signature is null ? null : new SignatureInfo(task.Signature.Alg, task.Signature.KeyId, task.Signature.Value));

    public static readonly JarvisTaskStatus[] ActiveStatuses =
    [
        JarvisTaskStatus.Dispatched, JarvisTaskStatus.Delivered, JarvisTaskStatus.Running, JarvisTaskStatus.WaitingForApproval,
    ];

    public static readonly JarvisTaskStatus[] ExecutingStatuses =
    [
        JarvisTaskStatus.Delivered, JarvisTaskStatus.Running, JarvisTaskStatus.WaitingForApproval,
    ];
}

public sealed record CreateTaskCommand(
    Guid? TargetDeviceId,
    string? Type,
    JsonElement? Payload,
    int? ExpiresInSeconds,
    int? MaxDurationSeconds,
    Guid? CorrelationId,
    string? IdempotencyKey,
    string Channel,
    Guid? TaskId = null,
    DateTimeOffset? ExpiresAt = null,
    SignatureInfo? Signature = null);

public sealed record CreateTaskResult(JarvisTask Task, bool Duplicate);

/// <summary>HU-026 / HU-027: caso de uso único para crear tareas por REST y por MQTT (docs/05-security/authorization.md).</summary>
public sealed class CreateTaskHandler(
    IJarvisDbContext db,
    DispatchSignal dispatchSignal,
    TimeProvider clock,
    IOptions<TaskOptions> options,
    ILogger<CreateTaskHandler> logger)
{
    public async Task<Result<CreateTaskResult>> HandleAsync(Principal principal, CreateTaskCommand command, CancellationToken cancellationToken)
    {
        // 2. Forma
        if (command.TargetDeviceId is null)
        {
            return AppError.Validation("targetDeviceId", "targetDeviceId is required.");
        }

        if (!DomainRules.IsValidTaskType(command.Type))
        {
            return AppError.Validation("type", "Must match ^[A-Z][A-Z0-9_]{1,63}$.");
        }

        string payloadJson;
        if (command.Payload is null || command.Payload.Value.ValueKind is JsonValueKind.Null or JsonValueKind.Undefined)
        {
            payloadJson = "{}";
        }
        else if (command.Payload.Value.ValueKind != JsonValueKind.Object)
        {
            return AppError.Validation("payload", "payload must be a JSON object.");
        }
        else
        {
            payloadJson = JsonText.Compact(command.Payload.Value);
        }

        if (DomainRules.Utf8Length(payloadJson) > DomainRules.MaxPayloadBytes)
        {
            return new AppError(ErrorCodes.PayloadTooLarge, "payload exceeds 16 KiB.");
        }

        var expiresIn = command.ExpiresInSeconds ?? DomainRules.DefaultExpiresInSeconds;
        if (expiresIn is < DomainRules.MinExpiresInSeconds or > DomainRules.MaxExpiresInSeconds)
        {
            return AppError.Validation("expiresInSeconds", $"Must be between {DomainRules.MinExpiresInSeconds} and {DomainRules.MaxExpiresInSeconds}.");
        }

        if (command.ExpiresAt is not null && command.ExpiresInSeconds is not null)
        {
            return AppError.Validation("expiresAt", "Send expiresAt or expiresInSeconds, not both.");
        }

        // Se valida aquí, y no solo en el dominio, para responder 400 en vez de un 500 por excepción
        // y para no llegar a mirar el device destino con una petición que ya es inválida.
        if (command.ExpiresAt is { } absolute)
        {
            var reference = clock.GetUtcNow();
            if (absolute < reference.AddSeconds(DomainRules.MinExpiresInSeconds)
                || absolute > reference.AddSeconds(DomainRules.MaxExpiresInSeconds))
            {
                return AppError.Validation(
                    "expiresAt",
                    $"Must be between {DomainRules.MinExpiresInSeconds} seconds and {DomainRules.MaxExpiresInSeconds / 86400} days from now.");
            }
        }

        // Una tarea firmada fija su propio taskId y expiresAt: ambos forman parte de la cadena que
        // el cliente firmó, así que el Cloud no puede generarlos (ADR-012).
        TaskSignature? signature = null;
        if (command.Signature is not null)
        {
            if (!DomainRules.IsWellFormedSignature(command.Signature.Alg, command.Signature.KeyId, command.Signature.Value))
            {
                return AppError.Validation("signature", "alg must be Ed25519, keyId [A-Za-z0-9_.:-]{1,64} and value 64 bytes in base64.");
            }

            if (command.TaskId is null || command.TaskId == Guid.Empty)
            {
                return AppError.Validation("taskId", "A signed task must carry the taskId that was signed.");
            }

            if (command.ExpiresAt is null)
            {
                return AppError.Validation("expiresAt", "A signed task must carry the expiresAt that was signed.");
            }

            signature = TaskSignature.Create(command.Signature.Alg, command.Signature.KeyId, command.Signature.Value);
        }
        else if (command.TaskId is not null)
        {
            return AppError.Validation("taskId", "taskId can only be chosen by the client on a signed task.");
        }

        if (command.MaxDurationSeconds is < 1 or > DomainRules.MaxDurationSeconds)
        {
            return AppError.Validation("maxDurationSeconds", $"Must be between 1 and {DomainRules.MaxDurationSeconds}.");
        }

        if (command.IdempotencyKey is null && principal.IsDevice)
        {
            return new AppError(ErrorCodes.IdempotencyKeyRequired, "Devices must send an idempotency key.");
        }

        if (command.IdempotencyKey is not null && !DomainRules.IsValidIdempotencyKey(command.IdempotencyKey))
        {
            return AppError.Validation("idempotencyKey", "Must be 1-100 characters [A-Za-z0-9_.:-].");
        }

        var targetId = command.TargetDeviceId.Value;
        var requestHash = SecretGenerator.Sha256(JsonText.Serialize(new
        {
            targetDeviceId = targetId,
            type = command.Type,
            payload = payloadJson,
            expiresInSeconds = command.ExpiresInSeconds,
            maxDurationSeconds = command.MaxDurationSeconds,
            taskId = command.TaskId,
            expiresAt = command.ExpiresAt,
            signature = command.Signature is null ? null : $"{command.Signature.KeyId}:{command.Signature.Value}",
        }));

        // 3. Idempotencia
        if (command.IdempotencyKey is not null)
        {
            var existing = await FindByIdempotencyKeyAsync(principal, command.IdempotencyKey, cancellationToken);
            if (existing is not null)
            {
                return existing.RequestHash is not null && existing.RequestHash.AsSpan().SequenceEqual(requestHash)
                    ? new CreateTaskResult(existing, Duplicate: true)
                    : new AppError(ErrorCodes.IdempotencyKeyReused, "The idempotency key was already used with different content.");
            }
        }

        var now = clock.GetUtcNow();

        // 4. Target del mismo owner
        var target = await db.Devices.FirstOrDefaultAsync(d => d.Id == targetId && d.OwnerId == principal.OwnerId, cancellationToken);
        if (target is null)
        {
            return await RejectAsync(principal, command, targetId, AppError.NotFound("Target device"), now, cancellationToken);
        }

        // 5. Grant para devices
        if (principal.IsDevice)
        {
            if (principal.Id == targetId)
            {
                return await RejectAsync(principal, command, targetId, new AppError(ErrorCodes.TaskNotGranted, "A device cannot create tasks for itself."), now, cancellationToken);
            }

            var granted = await db.DeviceTaskGrants.AnyAsync(
                g => g.SourceDeviceId == principal.Id && g.TargetDeviceId == targetId && g.TaskType == command.Type,
                cancellationToken);
            if (!granted)
            {
                return await RejectAsync(
                    principal,
                    command,
                    targetId,
                    new AppError(ErrorCodes.TaskNotGranted, $"Device is not allowed to create {command.Type} tasks for the target device."),
                    now,
                    cancellationToken);
            }
        }

        // 6. Target activo
        if (target.AdminState != DeviceAdminState.Active)
        {
            return await RejectAsync(principal, command, targetId, new AppError(ErrorCodes.TargetDeviceUnavailable, "Target device is disabled or revoked."), now, cancellationToken);
        }

        // 7. Tipo soportado
        if (!target.SupportedTaskTypes.Contains(command.Type!))
        {
            return await RejectAsync(principal, command, targetId, new AppError(ErrorCodes.TaskTypeNotSupported, $"Target device does not support {command.Type}."), now, cancellationToken);
        }

        if (command.TaskId is not null
            && await db.Tasks.AnyAsync(existing => existing.Id == command.TaskId.Value, cancellationToken))
        {
            // Mismo mensaje tanto si el id es de este owner como de otro: no se filtra su existencia.
            return new AppError(ErrorCodes.TaskIdInUse, "That taskId is already in use.");
        }

        JarvisTask task;
        try
        {
            task = JarvisTask.Create(
            command.TaskId ?? Guid.CreateVersion7(now),
            principal.OwnerId,
            principal.ToActor(),
            targetId,
            command.Type!,
            payloadJson,
            command.CorrelationId ?? Guid.CreateVersion7(now),
            command.IdempotencyKey,
            command.IdempotencyKey is null ? null : requestHash,
            expiresIn,
            command.MaxDurationSeconds,
            command.Channel,
            now,
            options.Value.MaxAttempts,
            command.ExpiresAt,
            signature);
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }

        try
        {
            db.Tasks.Add(task);
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (Exception exception) when (command.TaskId is not null && db.IsUniqueViolation(exception))
        {
            // Dos clientes eligiendo el mismo taskId a la vez: gana quien llegó primero.
            db.ResetTracking();
            return new AppError(ErrorCodes.TaskIdInUse, "That taskId is already in use.");
        }
        catch (Exception exception) when (command.IdempotencyKey is not null && db.IsUniqueViolation(exception))
        {
            // Carrera entre dos requests con la misma key: gana el primero.
            db.ResetTracking();
            var existing = await FindByIdempotencyKeyAsync(principal, command.IdempotencyKey, cancellationToken);
            if (existing is null)
            {
                throw;
            }

            return existing.RequestHash is not null && existing.RequestHash.AsSpan().SequenceEqual(requestHash)
                ? new CreateTaskResult(existing, Duplicate: true)
                : new AppError(ErrorCodes.IdempotencyKeyReused, "The idempotency key was already used with different content.");
        }

        logger.LogInformation("Task {TaskId} {TaskType} created for {TargetDeviceId} via {Channel}", task.Id, task.Type, targetId, command.Channel);
        dispatchSignal.Wake();
        return new CreateTaskResult(task, Duplicate: false);
    }

    private Task<JarvisTask?> FindByIdempotencyKeyAsync(Principal principal, string key, CancellationToken cancellationToken) =>
        principal.IsDevice
            ? db.Tasks.FirstOrDefaultAsync(
                t => t.OwnerId == principal.OwnerId && t.RequestedByType == ActorType.Device && t.SourceDeviceId == principal.Id && t.IdempotencyKey == key,
                cancellationToken)
            : db.Tasks.FirstOrDefaultAsync(
                t => t.OwnerId == principal.OwnerId && t.RequestedByType == ActorType.User && t.RequestedByUserId == principal.Id && t.IdempotencyKey == key,
                cancellationToken);

    private async Task<Result<CreateTaskResult>> RejectAsync(
        Principal principal,
        CreateTaskCommand command,
        Guid targetId,
        AppError error,
        DateTimeOffset now,
        CancellationToken cancellationToken)
    {
        db.AddEvent(new DomainEvent(
            EventTypes.TaskRejected,
            principal.OwnerId,
            now,
            DeviceId: principal.IsDevice ? principal.Id : null,
            Data: new Dictionary<string, object?> { ["type"] = command.Type, ["targetDeviceId"] = targetId, ["code"] = error.Code },
            Actor: principal.ToActor(),
            CorrelationId: command.CorrelationId));
        await db.SaveChangesAsync(cancellationToken);
        logger.LogInformation("Task creation rejected with {Code}", error.Code);
        return error;
    }
}

public sealed record TaskListQuery(
    IReadOnlyList<string> Statuses,
    Guid? TargetDeviceId,
    Guid? SourceDeviceId,
    string? Type,
    Guid? CorrelationId,
    DateTimeOffset? CreatedFrom,
    DateTimeOffset? CreatedTo,
    int? Limit,
    string? Cursor);

/// <summary>HU-033.</summary>
public sealed class TaskQueryHandler(IJarvisDbContext db)
{
    public async Task<Result<Page<TaskDto>>> ListAsync(Principal principal, TaskListQuery query, CancellationToken cancellationToken)
    {
        var tasks = db.Tasks.AsNoTracking().Where(t => t.OwnerId == principal.OwnerId);
        if (principal.IsDevice)
        {
            tasks = tasks.Where(t => t.SourceDeviceId == principal.Id || t.TargetDeviceId == principal.Id);
        }

        if (query.Statuses.Count > 0)
        {
            var statuses = new List<JarvisTaskStatus>();
            foreach (var value in query.Statuses)
            {
                if (!Enum.TryParse<JarvisTaskStatus>(value, ignoreCase: true, out var status) || int.TryParse(value, out _))
                {
                    return AppError.Validation("status", $"Unknown status '{value}'.");
                }

                statuses.Add(status);
            }

            tasks = tasks.Where(t => statuses.Contains(t.Status));
        }

        if (query.TargetDeviceId is { } targetId)
        {
            tasks = tasks.Where(t => t.TargetDeviceId == targetId);
        }

        if (query.SourceDeviceId is { } sourceId)
        {
            tasks = tasks.Where(t => t.SourceDeviceId == sourceId);
        }

        if (query.Type is not null)
        {
            tasks = tasks.Where(t => t.Type == query.Type);
        }

        if (query.CorrelationId is { } correlationId)
        {
            tasks = tasks.Where(t => t.CorrelationId == correlationId);
        }

        if (query.CreatedFrom is { } from)
        {
            tasks = tasks.Where(t => t.CreatedAt >= from);
        }

        if (query.CreatedTo is { } to)
        {
            tasks = tasks.Where(t => t.CreatedAt <= to);
        }

        if (query.Cursor is not null)
        {
            if (!KeysetCursor.TryDecode(query.Cursor, out var createdAt, out var id))
            {
                return AppError.Validation("cursor", "Invalid cursor.");
            }

            tasks = tasks.Where(t => t.CreatedAt < createdAt || (t.CreatedAt == createdAt && t.Id.CompareTo(id) < 0));
        }

        var limit = Limits.ClampLimit(query.Limit, 50, 200);
        var items = await tasks.OrderByDescending(t => t.CreatedAt).ThenByDescending(t => t.Id).Take(limit + 1).ToListAsync(cancellationToken);
        var next = items.Count > limit ? KeysetCursor.Encode(items[limit - 1].CreatedAt, items[limit - 1].Id) : null;
        return new Page<TaskDto>(items.Take(limit).Select(t => TaskMapper.ToDto(t, includeDetails: false)).ToList(), next);
    }

    public async Task<Result<TaskDto>> GetAsync(Principal principal, Guid id, CancellationToken cancellationToken)
    {
        var task = await db.Tasks.AsNoTracking().FirstOrDefaultAsync(t => t.Id == id && t.OwnerId == principal.OwnerId, cancellationToken);
        if (task is null || (principal.IsDevice && task.SourceDeviceId != principal.Id && task.TargetDeviceId != principal.Id))
        {
            return AppError.NotFound("Task");
        }

        return TaskMapper.ToDto(task, includeDetails: true);
    }
}

public sealed record ApprovalDecisionCommand(string? Decision, string? Nonce, DateTimeOffset? DecidedAt, SignatureInfo? Signature);

/// <summary>
/// HU-057: el owner decide en remoto una tarea que el device dejó en <c>WaitingForApproval</c>.
///
/// El Cloud <b>no</b> aprueba nada: registra la decisión firmada por el cliente y la retransmite al
/// device, que vuelve a verificar firma, nonce y frescura antes de aplicarla (ADR-012). Por eso un
/// 202 aquí significa "se transmitió", nunca "se aprobó".
/// </summary>
public sealed class DecideTaskApprovalHandler(
    IJarvisDbContext db,
    IDeviceMessagePublisher publisher,
    TimeProvider clock,
    ILogger<DecideTaskApprovalHandler> logger)
{
    public async Task<Result<TaskApproval>> HandleAsync(
        Principal principal,
        Guid taskId,
        ApprovalDecisionCommand command,
        CancellationToken cancellationToken)
    {
        if (!TryParseDecision(command.Decision, out var decision))
        {
            return AppError.Validation("decision", "Must be GRANTED or DENIED.");
        }

        if (!DomainRules.IsValidNonce(command.Nonce))
        {
            return AppError.Validation("nonce", "Must be 1-128 characters [A-Za-z0-9_.:-].");
        }

        if (command.DecidedAt is null)
        {
            return AppError.Validation("decidedAt", "decidedAt is required: it is part of the signed message.");
        }

        if (command.Signature is null
            || !DomainRules.IsWellFormedSignature(command.Signature.Alg, command.Signature.KeyId, command.Signature.Value))
        {
            // Sin firma la decisión no sirve de nada: el agente la ignoraría. Se corta aquí.
            return AppError.Validation("signature", "A remote approval decision must be signed (Ed25519).");
        }

        var task = await db.Tasks.FirstOrDefaultAsync(t => t.Id == taskId && t.OwnerId == principal.OwnerId, cancellationToken);
        if (task is null)
        {
            return AppError.NotFound("Task");
        }

        if (task.Status != JarvisTaskStatus.WaitingForApproval)
        {
            return new AppError(ErrorCodes.InvalidStateTransition, $"The task is {task.Status}, not WaitingForApproval.");
        }

        var now = clock.GetUtcNow();
        TaskApproval approval;
        try
        {
            approval = TaskApproval.Create(
                Guid.CreateVersion7(now),
                task,
                principal.ToActor(),
                decision,
                command.DecidedAt.Value,
                command.Nonce!,
                TaskSignature.Create(command.Signature.Alg, command.Signature.KeyId, command.Signature.Value),
                now);
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }

        try
        {
            db.TaskApprovals.Add(approval);
            await db.SaveChangesAsync(cancellationToken);
        }
        catch (Exception exception) when (db.IsUniqueViolation(exception))
        {
            db.ResetTracking();
            var existing = await db.TaskApprovals.AsNoTracking()
                .FirstOrDefaultAsync(a => a.TaskId == taskId && a.OwnerId == principal.OwnerId, cancellationToken);
            return existing is not null
                ? new AppError(ErrorCodes.ApprovalAlreadyDecided, "That task already has a decision.")
                : new AppError(ErrorCodes.NonceReused, "That nonce was already used.");
        }

        await publisher.PublishAsync(
            task.TargetDeviceId,
            MessageTypes.TaskApprovalDecision,
            new TaskApprovalDecisionData(
                task.Id,
                decision.ToString().ToUpperInvariant(),
                new RequestedByInfo(approval.DecidedByType.ToString(), approval.DecidedByUserId ?? approval.DecidedByDeviceId!.Value),
                approval.DecidedAt,
                approval.Nonce,
                new SignatureInfo(approval.SignatureAlg, approval.SignatureKeyId, approval.SignatureValue)),
            task.CorrelationId,
            null,
            cancellationToken);

        logger.LogInformation(
            "Approval decision {Decision} for task {TaskId} relayed to {DeviceId} (key {KeyId})",
            decision,
            task.Id,
            task.TargetDeviceId,
            approval.SignatureKeyId);
        return approval;
    }

    private static bool TryParseDecision(string? value, out ApprovalDecision decision)
    {
        decision = ApprovalDecision.Granted;
        switch (value)
        {
            case "GRANTED":
                return true;
            case "DENIED":
                decision = ApprovalDecision.Denied;
                return true;
            default:
                return false;
        }
    }
}

public sealed record CancelTaskResult(JarvisTask Task, bool Immediate);

/// <summary>HU-032: cancelación por REST (owner o device origen) y por MQTT (device origen).</summary>
public sealed class CancelTaskHandler(
    IJarvisDbContext db,
    IDeviceMessagePublisher publisher,
    TimeProvider clock,
    ILogger<CancelTaskHandler> logger)
{
    public async Task<Result<CancelTaskResult>> HandleAsync(Principal principal, Guid taskId, CancellationToken cancellationToken)
    {
        var task = await db.Tasks.FirstOrDefaultAsync(t => t.Id == taskId && t.OwnerId == principal.OwnerId, cancellationToken);
        if (task is null)
        {
            return AppError.NotFound("Task");
        }

        if (principal.IsDevice && task.SourceDeviceId != principal.Id)
        {
            return task.TargetDeviceId == principal.Id
                ? AppError.Forbidden("Only the owner or the source device can cancel a task.")
                : AppError.NotFound("Task");
        }

        CancelOutcome outcome;
        try
        {
            outcome = task.RequestCancel(principal.ToActor(), clock.GetUtcNow());
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }

        if (outcome != CancelOutcome.AlreadyRequested)
        {
            await db.SaveChangesAsync(cancellationToken);
        }

        var mustNotifyDevice = outcome == CancelOutcome.CancelRequested
            || (outcome == CancelOutcome.CancelledImmediately && task.AttemptCount > 0);
        if (mustNotifyDevice)
        {
            await publisher.PublishAsync(
                task.TargetDeviceId,
                MessageTypes.TaskCancel,
                new TaskCancelData(task.Id, TaskErrorCodes.UserRequest),
                task.CorrelationId,
                null,
                cancellationToken);
        }

        logger.LogInformation("Cancel of task {TaskId}: {Outcome}", task.Id, outcome);
        return new CancelTaskResult(task, outcome == CancelOutcome.CancelledImmediately);
    }
}
