using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Domain.Tasks;

namespace Jarvis.Domain.Tests;

/// <summary>
/// ADR-012 y HU-057. Lo que se comprueba aquí es la <b>forma</b> y las reglas de estado: el Cloud
/// no verifica firmas (no tiene la clave pública) y no debe dar la sensación de que lo hace.
/// </summary>
public sealed class TaskSigningTests
{
    private static readonly DateTimeOffset T0 = new(2026, 9, 23, 10, 0, 0, TimeSpan.Zero);
    private static readonly Guid Owner = Guid.CreateVersion7();
    private static readonly Guid User = Guid.CreateVersion7();
    private static readonly Guid Target = Guid.CreateVersion7();

    /// <summary>64 bytes en base64: la forma que tiene una firma Ed25519.</summary>
    private static string SignatureValue(byte fill = 1) => Convert.ToBase64String(Enumerable.Repeat(fill, 64).ToArray());

    private static TaskSignature Signature() => TaskSignature.Create("Ed25519", "owner-laptop-1", SignatureValue());

    private static JarvisTask NewTask(
        DateTimeOffset? expiresAt = null,
        TaskSignature? signature = null,
        int expiresIn = 600) =>
        JarvisTask.Create(
            Guid.CreateVersion7(), Owner, Actor.User(User), Target, "CLOSE_APP", """{"appId":"notepad","force":true}""",
            Guid.CreateVersion7(), null, null, expiresIn, null, "rest", T0, 5, expiresAt, signature);

    [Fact]
    public void A_signature_needs_the_right_algorithm_key_id_and_64_bytes()
    {
        Assert.True(DomainRules.IsWellFormedSignature("Ed25519", "owner-laptop-1", SignatureValue()));
        Assert.True(DomainRules.IsWellFormedSignature("ed25519", "owner-laptop-1", SignatureValue()));

        Assert.False(DomainRules.IsWellFormedSignature("RS256", "owner-laptop-1", SignatureValue()));
        Assert.False(DomainRules.IsWellFormedSignature("Ed25519", "clave con espacios", SignatureValue()));
        Assert.False(DomainRules.IsWellFormedSignature("Ed25519", new string('k', 65), SignatureValue()));
        Assert.False(DomainRules.IsWellFormedSignature("Ed25519", "owner-laptop-1", "no es base64!!"));
        // 32 bytes es una clave, no una firma.
        Assert.False(DomainRules.IsWellFormedSignature("Ed25519", "owner-laptop-1", Convert.ToBase64String(new byte[32])));
        Assert.False(DomainRules.IsWellFormedSignature("Ed25519", "owner-laptop-1", null));
        Assert.False(DomainRules.IsWellFormedSignature(null, "owner-laptop-1", SignatureValue()));
    }

    [Fact]
    public void A_signed_task_keeps_the_expiry_that_was_signed()
    {
        var expiresAt = T0.AddMinutes(7);
        var task = NewTask(expiresAt, Signature());

        // Si el Cloud recalculara expiresAt, la firma dejaría de validar en el agente.
        Assert.Equal(expiresAt, task.ExpiresAt);
        Assert.True(task.IsSigned);
        Assert.Equal("owner-laptop-1", task.Signature!.KeyId);
    }

    [Fact]
    public void An_unsigned_task_still_works_and_derives_its_expiry()
    {
        var task = NewTask(expiresIn: 300);

        Assert.Equal(T0.AddSeconds(300), task.ExpiresAt);
        Assert.False(task.IsSigned);
        Assert.Null(task.Signature);
    }

    [Fact]
    public void An_expiry_outside_the_allowed_window_is_refused()
    {
        Assert.Throws<DomainException>(() => NewTask(T0.AddSeconds(1)));
        Assert.Throws<DomainException>(() => NewTask(T0.AddDays(8)));
        Assert.Throws<DomainException>(() => NewTask(T0.AddMinutes(-1)));
    }

    [Fact]
    public void Creating_a_signed_task_records_which_key_signed_it()
    {
        var task = NewTask(T0.AddMinutes(5), Signature());

        var created = Assert.Single(task.DomainEvents, e => e.Type == EventTypes.TaskCreated);
        Assert.Equal(true, created.Data!["signed"]);
        Assert.Equal("owner-laptop-1", created.Data["signatureKeyId"]);
    }

    [Fact]
    public void A_malformed_signature_never_becomes_a_task_signature()
    {
        Assert.Throws<DomainException>(() => TaskSignature.Create("Ed25519", "owner-laptop-1", "xx"));
        Assert.Throws<DomainException>(() => TaskSignature.Create("HS256", "owner-laptop-1", SignatureValue()));
    }

    // ---- HU-057: decisión de aprobación ------------------------------------------

    private static JarvisTask WaitingForApproval()
    {
        var task = NewTask();
        task.Dispatch(T0.AddSeconds(1));
        task.Acknowledge(T0.AddSeconds(2));
        task.RequireApproval("R3 requires approval", T0.AddSeconds(3));
        task.ClearDomainEvents();
        return task;
    }

    private static TaskApproval Decide(
        JarvisTask task,
        ApprovalDecision decision = ApprovalDecision.Granted,
        DateTimeOffset? decidedAt = null,
        string nonce = "nonce-0001",
        DateTimeOffset? now = null) =>
        TaskApproval.Create(
            Guid.CreateVersion7(),
            task,
            Actor.User(User),
            decision,
            decidedAt ?? T0.AddSeconds(10),
            nonce,
            Signature(),
            now ?? T0.AddSeconds(11));

    [Fact]
    public void A_decision_keeps_what_the_client_signed()
    {
        var task = WaitingForApproval();
        var decidedAt = T0.AddSeconds(10);

        var approval = Decide(task, decidedAt: decidedAt);

        // El agente reconstruye el mensaje firmado con estos campos: si el Cloud los tocara
        // (por ejemplo, guardando su propia hora), la firma dejaría de cuadrar.
        Assert.Equal(decidedAt, approval.DecidedAt);
        Assert.Equal("nonce-0001", approval.Nonce);
        Assert.Equal(task.Id, approval.TaskId);
        Assert.Equal(ApprovalDecision.Granted, approval.Decision);
        Assert.Equal(Signature(), approval.Signature);
    }

    [Fact]
    public void Deciding_records_that_the_decision_was_sent_not_that_it_was_applied()
    {
        // APPROVAL_GRANTED lo emite el device cuando de verdad la aplica. Si el Cloud lo emitiera
        // aquí, una firma inválida quedaría registrada como una aprobación concedida.
        foreach (var decision in new[] { ApprovalDecision.Granted, ApprovalDecision.Denied })
        {
            var sent = Decide(WaitingForApproval(), decision).DomainEvents[0];
            Assert.Equal(EventTypes.ApprovalDecisionSent, sent.Type);
            Assert.Equal(decision.ToString().ToUpperInvariant(), sent.Data!["decision"]);
            Assert.Equal("owner-laptop-1", sent.Data["signatureKeyId"]);
        }
    }

    [Fact]
    public void A_task_that_is_not_waiting_cannot_be_decided()
    {
        var pending = NewTask();

        Assert.Throws<DomainException>(() => Decide(pending));
    }

    [Fact]
    public void A_stale_decision_is_refused_here_instead_of_dying_silently_at_the_agent()
    {
        var task = WaitingForApproval();
        var old = T0.AddSeconds(-DomainRules.ApprovalDecisionMaxAgeSeconds - 60);

        Assert.Throws<DomainException>(() => Decide(task, decidedAt: old, now: T0));
        // Y tampoco vale una decisión "del futuro": el reloj del cliente puede ir adelantado.
        Assert.Throws<DomainException>(() => Decide(task, decidedAt: T0.AddHours(1), now: T0));
    }

    [Fact]
    public void A_malformed_nonce_is_refused()
    {
        var task = WaitingForApproval();

        Assert.Throws<DomainException>(() => Decide(task, nonce: string.Empty));
        Assert.Throws<DomainException>(() => Decide(task, nonce: new string('n', DomainRules.MaxNonceLength + 1)));
        Assert.Throws<DomainException>(() => Decide(task, nonce: "nonce con espacios"));
    }
}
