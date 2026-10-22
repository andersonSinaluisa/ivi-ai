using Jarvis.Domain.Common;
using Jarvis.Domain.Events;

namespace Jarvis.Domain.Tasks;

public enum ApprovalDecision
{
    Granted,
    Denied,
}

/// <summary>
/// HU-057: decisión de aprobación tomada en remoto por el cliente del owner y firmada por él
/// (ADR-012). El Cloud la registra para tener auditoría y para poder detectar un reenvío, pero
/// <b>no</b> es quien decide: el device vuelve a verificar la firma, el nonce y la frescura antes
/// de aplicarla. Una decisión guardada aquí es "lo que el owner dijo", no "lo que el agente hizo".
/// </summary>
public sealed class TaskApproval : Entity
{
    private TaskApproval()
    {
    }

    public Guid OwnerId { get; private set; }

    public Guid TaskId { get; private set; }

    public ApprovalDecision Decision { get; private set; }

    public ActorType DecidedByType { get; private set; }

    public Guid? DecidedByUserId { get; private set; }

    public Guid? DecidedByDeviceId { get; private set; }

    /// <summary>Momento que el cliente firmó, no el de llegada: es lo que el agente comprueba.</summary>
    public DateTimeOffset DecidedAt { get; private set; }

    /// <summary>Único por decisión; el agente lo consume una sola vez (anti-replay).</summary>
    public string Nonce { get; private set; } = string.Empty;

    public string SignatureAlg { get; private set; } = string.Empty;

    public string SignatureKeyId { get; private set; } = string.Empty;

    public string SignatureValue { get; private set; } = string.Empty;

    public DateTimeOffset CreatedAt { get; private set; }

    public TaskSignature Signature => new(SignatureAlg, SignatureKeyId, SignatureValue);

    public static TaskApproval Create(
        Guid id,
        JarvisTask task,
        Actor decidedBy,
        ApprovalDecision decision,
        DateTimeOffset decidedAt,
        string nonce,
        TaskSignature signature,
        DateTimeOffset now)
    {
        if (task.Status != JarvisTaskStatus.WaitingForApproval)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "The task is not waiting for approval.");
        }

        if (!DomainRules.IsValidNonce(nonce))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Malformed nonce.");
        }

        if (decidedBy.Type == ActorType.System || decidedBy.Id is null)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "An approval must be decided by a user or a device.");
        }

        // Fuera de la ventana de frescura el agente la descartaría igualmente: mejor decirlo aquí,
        // donde el cliente aún puede corregir su reloj, que dejar la tarea colgada sin explicación.
        var age = (now - decidedAt).Duration().TotalSeconds;
        if (age > DomainRules.ApprovalDecisionMaxAgeSeconds)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "The decision is outside its freshness window.");
        }

        var approval = new TaskApproval
        {
            Id = id,
            OwnerId = task.OwnerId,
            TaskId = task.Id,
            Decision = decision,
            DecidedByType = decidedBy.Type,
            DecidedByUserId = decidedBy.Type == ActorType.User ? decidedBy.Id : null,
            DecidedByDeviceId = decidedBy.Type == ActorType.Device ? decidedBy.Id : null,
            DecidedAt = decidedAt,
            Nonce = nonce,
            SignatureAlg = signature.Alg,
            SignatureKeyId = signature.KeyId,
            SignatureValue = signature.Value,
            CreatedAt = now,
        };

        // Es APPROVAL_DECISION_SENT y no APPROVAL_GRANTED/DENIED a propósito: lo que ocurrió aquí es
        // que el owner firmó y el Cloud transmitió. Si la firma no cuadra, el device no la aplicará
        // y no habrá ningún APPROVAL_GRANTED después: la diferencia entre ambos eventos es lo que
        // permite ver esos casos en la auditoría.
        approval.Raise(new DomainEvent(
            EventTypes.ApprovalDecisionSent,
            task.OwnerId,
            now,
            DeviceId: task.TargetDeviceId,
            TaskId: task.Id,
            Data: new Dictionary<string, object?>
            {
                ["decision"] = decision.ToString().ToUpperInvariant(),
                ["nonce"] = nonce,
                ["signatureKeyId"] = signature.KeyId,
                ["decidedAt"] = decidedAt,
                ["channel"] = "rest",
            },
            Actor: decidedBy));
        return approval;
    }
}
