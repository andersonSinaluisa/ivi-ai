using Jarvis.Domain.Common;
using Jarvis.Domain.Events;
using Jarvis.Domain.Tasks;

namespace Jarvis.Domain.Tests;

/// <summary>Cobertura de T1-T17 y de la regla de avance monótono (docs/02-domain/task-state-machine.md).</summary>
public sealed class TaskStateMachineTests
{
    private static readonly DateTimeOffset T0 = new(2026, 9, 19, 15, 0, 0, TimeSpan.Zero);
    private static readonly Guid Owner = Guid.CreateVersion7();
    private static readonly Guid Source = Guid.CreateVersion7();
    private static readonly Guid Target = Guid.CreateVersion7();

    private static JarvisTask NewTask(int expiresIn = 600, int? maxDuration = null, int maxAttempts = 5)
    {
        var task = JarvisTask.Create(
            Guid.CreateVersion7(), Owner, Actor.Device(Source), Target, "OPEN_CHROME", """{"url":"https://example.com"}""",
            Guid.CreateVersion7(), "btn1-000001", [1, 2, 3], expiresIn, maxDuration, "mqtt", T0, maxAttempts);
        return task;
    }

    private static JarvisTask Dispatched()
    {
        var task = NewTask();
        task.Dispatch(T0.AddSeconds(1));
        task.ClearDomainEvents();
        return task;
    }

    private static List<string> Events(JarvisTask task) => task.DomainEvents.Select(e => e.Type).ToList();

    [Fact]
    public void T1_create_is_pending_with_task_created_event()
    {
        var task = NewTask();
        Assert.Equal(JarvisTaskStatus.Pending, task.Status);
        Assert.Equal([EventTypes.TaskCreated], Events(task));
        Assert.Equal(T0.AddSeconds(600), task.ExpiresAt);
        Assert.Equal(Source, task.SourceDeviceId);
        Assert.All(task.DomainEvents, e => Assert.Equal(task.CorrelationId, e.CorrelationId));
    }

    [Theory]
    [InlineData("open_chrome")]
    [InlineData("A")]
    [InlineData("")]
    [InlineData("1ABC")]
    public void Create_rejects_invalid_task_type(string type) =>
        Assert.Throws<DomainException>(() => JarvisTask.Create(
            Guid.NewGuid(), Owner, Actor.User(Owner), Target, type, "{}", Guid.NewGuid(), null, null, 600, null, "rest", T0));

    [Fact]
    public void Create_rejects_payload_over_16_kib()
    {
        var payload = "{\"x\":\"" + new string('a', 16 * 1024) + "\"}";
        var exception = Assert.Throws<DomainException>(() => JarvisTask.Create(
            Guid.NewGuid(), Owner, Actor.User(Owner), Target, "OPEN_CHROME", payload, Guid.NewGuid(), null, null, 600, null, "rest", T0));
        Assert.Equal(DomainErrorCodes.PayloadTooLarge, exception.Code);
    }

    [Fact]
    public void Create_rejects_device_targeting_itself() =>
        Assert.Throws<DomainException>(() => JarvisTask.Create(
            Guid.NewGuid(), Owner, Actor.Device(Target), Target, "OPEN_CHROME", "{}", Guid.NewGuid(), "k", null, 600, null, "mqtt", T0));

    [Fact]
    public void T2_dispatch_increments_attempt()
    {
        var task = NewTask();
        task.ClearDomainEvents();
        task.Dispatch(T0.AddSeconds(1));
        Assert.Equal(JarvisTaskStatus.Dispatched, task.Status);
        Assert.Equal(1, task.AttemptCount);
        Assert.Equal([EventTypes.TaskDispatched], Events(task));
        Assert.Equal(1, task.DomainEvents[0].Data!["attempt"]);
    }

    [Fact]
    public void Dispatch_requires_pending()
    {
        var task = Dispatched();
        Assert.Throws<DomainException>(() => task.Dispatch(T0));
    }

    [Fact]
    public void T3_cancel_pending_is_immediate()
    {
        var task = NewTask();
        task.ClearDomainEvents();
        Assert.Equal(CancelOutcome.CancelledImmediately, task.RequestCancel(Actor.User(Owner), T0));
        Assert.Equal(JarvisTaskStatus.Cancelled, task.Status);
        Assert.Equal(TaskErrorCodes.UserRequest, task.ErrorCode);
        Assert.NotNull(task.CompletedAt);
        Assert.Equal([EventTypes.TaskCancelled], Events(task));
    }

    [Fact]
    public void T4_pending_expires_after_expires_at()
    {
        var task = NewTask(expiresIn: 60);
        task.ClearDomainEvents();
        Assert.False(task.ExpireIfDue(T0.AddSeconds(60)));
        Assert.True(task.ExpireIfDue(T0.AddSeconds(61)));
        Assert.Equal(JarvisTaskStatus.Expired, task.Status);
        Assert.Equal(TaskErrorCodes.NotDelivered, task.ErrorCode);
        Assert.Equal([EventTypes.TaskExpired], Events(task));
    }

    [Fact]
    public void T5_ack_timeout_reverts_to_pending()
    {
        var task = Dispatched();
        Assert.True(task.HandleAckTimeout(T0.AddSeconds(40)));
        Assert.Equal(JarvisTaskStatus.Pending, task.Status);
        Assert.Equal("ack_timeout", task.StatusReason);
        Assert.Empty(task.DomainEvents);
    }

    [Fact]
    public void T6_ack_timeout_after_max_attempts_fails_delivery()
    {
        var task = NewTask(maxAttempts: 2);
        task.Dispatch(T0);
        task.HandleAckTimeout(T0);
        task.Dispatch(T0);
        task.ClearDomainEvents();
        Assert.True(task.HandleAckTimeout(T0));
        Assert.Equal(JarvisTaskStatus.Failed, task.Status);
        Assert.Equal(TaskErrorCodes.DeliveryFailed, task.ErrorCode);
        Assert.Equal([EventTypes.TaskFailed], Events(task));
    }

    [Fact]
    public void T7_rejected_fails_with_device_code()
    {
        var task = Dispatched();
        Assert.Equal(TaskUpdateOutcome.Applied, task.Reject(new TaskError(TaskErrorCodes.NotPermitted, "nope"), T0));
        Assert.Equal(JarvisTaskStatus.Failed, task.Status);
        Assert.Equal(TaskErrorCodes.NotPermitted, task.ErrorCode);
    }

    [Fact]
    public void T8_dispatched_expires_only_after_grace()
    {
        var task = NewTask(expiresIn: 60);
        task.Dispatch(T0);
        Assert.False(task.ExpireIfDue(T0.AddSeconds(61)));
        Assert.True(task.ExpireIfDue(T0.AddSeconds(91)));
        Assert.Equal(JarvisTaskStatus.Expired, task.Status);
        Assert.Equal("Dispatched", task.DomainEvents.Last().Data!["lastStatus"]);
    }

    [Fact]
    public void T9_ack_delivers()
    {
        var task = Dispatched();
        Assert.Equal(TaskUpdateOutcome.Applied, task.Acknowledge(T0.AddSeconds(2)));
        Assert.Equal(JarvisTaskStatus.Delivered, task.Status);
        Assert.Equal(T0.AddSeconds(2), task.DeliveredAt);
        Assert.Equal([EventTypes.TaskDelivered], Events(task));
    }

    [Fact]
    public void T10_started_from_dispatched_fills_delivery()
    {
        var task = Dispatched();
        Assert.Equal(TaskUpdateOutcome.Applied, task.Start(T0.AddSeconds(3)));
        Assert.Equal(JarvisTaskStatus.Running, task.Status);
        Assert.Equal(T0.AddSeconds(3), task.DeliveredAt);
        Assert.Equal(T0.AddSeconds(3), task.StartedAt);
        Assert.Equal([EventTypes.TaskDelivered, EventTypes.TaskStarted], Events(task));
    }

    [Fact]
    public void T11_T10_approval_roundtrip()
    {
        var task = Dispatched();
        task.Acknowledge(T0);
        task.Start(T0);
        task.ClearDomainEvents();
        Assert.Equal(TaskUpdateOutcome.Applied, task.RequireApproval("confirm", T0));
        Assert.Equal(JarvisTaskStatus.WaitingForApproval, task.Status);
        Assert.Equal(TaskUpdateOutcome.Applied, task.Start(T0));
        Assert.Equal(JarvisTaskStatus.Running, task.Status);
        Assert.Equal([EventTypes.ApprovalRequested, EventTypes.ApprovalGranted], Events(task));
    }

    [Fact]
    public void T12_completed_sets_result()
    {
        var task = Dispatched();
        task.Acknowledge(T0);
        task.Start(T0);
        task.ClearDomainEvents();
        Assert.Equal(TaskUpdateOutcome.Applied, task.Complete("""{"outcome":"SUCCESS"}""", T0.AddSeconds(5)));
        Assert.Equal(JarvisTaskStatus.Completed, task.Status);
        Assert.Equal(T0.AddSeconds(5), task.CompletedAt);
        Assert.Contains("SUCCESS", task.Result, StringComparison.Ordinal);
        Assert.Equal([EventTypes.TaskCompleted], Events(task));
    }

    [Fact]
    public void T12_completed_from_dispatched_does_not_invent_started()
    {
        var task = Dispatched();
        task.Complete("{}", T0);
        Assert.Equal([EventTypes.TaskDelivered, EventTypes.TaskCompleted], Events(task));
        Assert.Null(task.StartedAt);
    }

    [Fact]
    public void Completed_with_oversized_result_fails()
    {
        var task = Dispatched();
        task.Complete("{\"x\":\"" + new string('a', 33 * 1024) + "\"}", T0);
        Assert.Equal(JarvisTaskStatus.Failed, task.Status);
        Assert.Equal(TaskErrorCodes.ResultTooLarge, task.ErrorCode);
    }

    [Fact]
    public void T13_failed_from_running()
    {
        var task = Dispatched();
        task.Start(T0);
        task.Fail(new TaskError(TaskErrorCodes.ExecutionError, "boom"), T0);
        Assert.Equal(JarvisTaskStatus.Failed, task.Status);
        Assert.Equal("boom", task.ErrorMessage);
    }

    [Fact]
    public void T14_cancelled_with_approval_denied_emits_both_events()
    {
        var task = Dispatched();
        task.Acknowledge(T0);
        task.RequireApproval(null, T0);
        task.ClearDomainEvents();
        task.ConfirmCancelled(TaskErrorCodes.ApprovalDenied, T0);
        Assert.Equal(JarvisTaskStatus.Cancelled, task.Status);
        Assert.Equal([EventTypes.ApprovalDenied, EventTypes.TaskCancelled], Events(task));
    }

    [Theory]
    [InlineData(JarvisTaskStatus.Dispatched, JarvisTaskStatus.Pending)]
    [InlineData(JarvisTaskStatus.Delivered, JarvisTaskStatus.Failed)]
    [InlineData(JarvisTaskStatus.Running, JarvisTaskStatus.Failed)]
    [InlineData(JarvisTaskStatus.WaitingForApproval, JarvisTaskStatus.Failed)]
    public void T5_T15_reconciliation_when_missing_on_device(JarvisTaskStatus current, JarvisTaskStatus expected)
    {
        var task = InStatus(current);
        Assert.True(task.HandleMissingOnDevice(T0));
        Assert.Equal(expected, task.Status);
        if (expected == JarvisTaskStatus.Failed)
        {
            Assert.Equal(TaskErrorCodes.DeviceLost, task.ErrorCode);
        }
        else
        {
            Assert.Equal("not_on_device", task.StatusReason);
        }
    }

    [Fact]
    public void T16_execution_timeout_uses_max_duration()
    {
        var task = NewTask(maxDuration: 120);
        task.Dispatch(T0);
        task.Acknowledge(T0);
        Assert.False(task.TimeoutIfDue(T0.AddSeconds(120)));
        Assert.True(task.TimeoutIfDue(T0.AddSeconds(121)));
        Assert.Equal(TaskErrorCodes.ExecutionTimeout, task.ErrorCode);
    }

    [Fact]
    public void T16_execution_timeout_defaults_to_24_hours()
    {
        var task = Dispatched();
        task.Acknowledge(T0);
        Assert.False(task.TimeoutIfDue(T0.AddHours(23)));
        Assert.True(task.TimeoutIfDue(T0.AddHours(24).AddSeconds(1)));
    }

    [Fact]
    public void T17_cancel_active_task_only_requests()
    {
        var task = Dispatched();
        Assert.Equal(CancelOutcome.CancelRequested, task.RequestCancel(Actor.User(Owner), T0));
        Assert.Equal(JarvisTaskStatus.Dispatched, task.Status);
        Assert.NotNull(task.CancelRequestedAt);
        Assert.Equal(CancelOutcome.AlreadyRequested, task.RequestCancel(Actor.User(Owner), T0));
        Assert.Single(task.DomainEvents, e => e.Type == EventTypes.TaskCancelRequested);
    }

    [Fact]
    public void Cancel_terminal_is_invalid_transition()
    {
        var task = Dispatched();
        task.Complete("{}", T0);
        var exception = Assert.Throws<DomainException>(() => task.RequestCancel(Actor.User(Owner), T0));
        Assert.Equal(DomainErrorCodes.InvalidStateTransition, exception.Code);
    }

    [Fact]
    public void Duplicate_messages_produce_single_event()
    {
        var task = Dispatched();
        Assert.Equal(TaskUpdateOutcome.Applied, task.Acknowledge(T0));
        Assert.Equal(TaskUpdateOutcome.Duplicate, task.Acknowledge(T0));
        Assert.Equal(TaskUpdateOutcome.Applied, task.Start(T0));
        Assert.Equal(TaskUpdateOutcome.Duplicate, task.Start(T0));
        Assert.Equal(TaskUpdateOutcome.Stale, task.Acknowledge(T0));
        Assert.Equal([EventTypes.TaskDelivered, EventTypes.TaskStarted], Events(task));
    }

    [Fact]
    public void Late_terminal_update_is_ignored()
    {
        var task = Dispatched();
        task.ConfirmCancelled(TaskErrorCodes.UserRequest, T0);
        task.ClearDomainEvents();
        Assert.Equal(TaskUpdateOutcome.Terminal, task.Complete("{}", T0));
        Assert.Equal(JarvisTaskStatus.Cancelled, task.Status);
        Assert.Empty(task.DomainEvents);
    }

    [Fact]
    public void Message_on_never_dispatched_task_is_security_violation()
    {
        var task = NewTask();
        Assert.Equal(TaskUpdateOutcome.SecurityViolation, task.Acknowledge(T0));
        Assert.Equal(JarvisTaskStatus.Pending, task.Status);
    }

    [Fact]
    public void Message_on_reverted_pending_task_is_processed()
    {
        var task = Dispatched();
        task.HandleAckTimeout(T0);
        Assert.Equal(JarvisTaskStatus.Pending, task.Status);
        Assert.Equal(TaskUpdateOutcome.Applied, task.Acknowledge(T0));
        Assert.Equal(JarvisTaskStatus.Delivered, task.Status);
    }

    [Fact]
    public void Random_message_sequences_never_decrease_rank()
    {
        var random = new Random(42);
        for (var run = 0; run < 500; run++)
        {
            var task = Dispatched();
            var rank = JarvisTask.Rank(task.Status);
            for (var step = 0; step < 12; step++)
            {
                _ = random.Next(7) switch
                {
                    0 => task.Acknowledge(T0),
                    1 => task.Start(T0),
                    2 => task.RequireApproval("r", T0),
                    3 => task.Complete("{}", T0),
                    4 => task.Fail(new TaskError("EXECUTION_ERROR", null), T0),
                    5 => task.ConfirmCancelled("USER_REQUEST", T0),
                    _ => task.ReportProgress(50, "half", T0),
                };
                var newRank = JarvisTask.Rank(task.Status);
                Assert.True(newRank >= rank, $"rank went from {rank} to {newRank}");
                rank = newRank;
            }

            Assert.True(task.DomainEvents.Count(e => e.Type is EventTypes.TaskCompleted or EventTypes.TaskFailed or EventTypes.TaskCancelled) <= 1);
        }
    }

    private static JarvisTask InStatus(JarvisTaskStatus status)
    {
        var task = Dispatched();
        switch (status)
        {
            case JarvisTaskStatus.Delivered:
                task.Acknowledge(T0);
                break;
            case JarvisTaskStatus.Running:
                task.Start(T0);
                break;
            case JarvisTaskStatus.WaitingForApproval:
                task.RequireApproval("x", T0);
                break;
        }

        return task;
    }
}
