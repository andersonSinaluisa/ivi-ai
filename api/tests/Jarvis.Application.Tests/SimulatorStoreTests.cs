using Jarvis.Contracts.Mqtt;
using Jarvis.Simulators;

namespace Jarvis.Application.Tests;

/// <summary>Inbox/outbox del Windows Simulator: la base de la no-duplicación (W4-W7).</summary>
public sealed class SimulatorStoreTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "jarvis-sim-tests", Guid.NewGuid().ToString("N"));

    [Fact]
    public void Store_survives_restart_and_tracks_executions()
    {
        var taskId = Guid.CreateVersion7();
        var path = Path.Combine(_directory, "store.db");
        var ack = MqttEnvelope.Create(MessageTypes.TaskAck, Guid.NewGuid(), new TaskRefData(taskId), DateTimeOffset.UtcNow);

        using (var store = new LocalTaskStore(path))
        {
            store.Insert(taskId, null, "Received", ack);
            store.Insert(taskId, null, "Received", ack); // duplicado ignorado
            store.Update(taskId, "Starting", ack);
            store.IncrementExecutions(taskId);
            Assert.Equal([taskId], store.OpenTaskIds());
        }

        using (var reopened = new LocalTaskStore(path))
        {
            Assert.Equal("Starting", reopened.Get(taskId)!.Status);
            Assert.Equal(1, reopened.ExecutionCount(taskId));
            reopened.MarkSettled(taskId);
            Assert.Empty(reopened.OpenTaskIds());
            Assert.NotNull(reopened.Get(taskId)!.LastMessage);
        }
    }

    public void Dispose()
    {
        Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }
}
