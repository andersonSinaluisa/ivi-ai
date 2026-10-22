namespace Jarvis.Contracts.Mqtt;

public static class TopicChannels
{
    public const string Tasks = "tasks";
    public const string Notifications = "notifications";
    public const string Control = "control";
    public const string Requests = "requests";
    public const string Presence = "presence";
    public const string Events = "events";
}

public static class TopicDirections
{
    public const string Up = "up";
    public const string Down = "down";
}

/// <summary>Layout de topics v1: <c>jarvis/v1/devices/{deviceId}/{up|down}/{channel}</c>.</summary>
public static class MqttTopics
{
    public const string Prefix = "jarvis/v1/devices/";

    /// <summary>Filtro de suscripción del cloud client.</summary>
    public const string CloudSubscription = "jarvis/v1/devices/+/up/#";

    public static string DeviceSubscription(Guid deviceId) => $"{Prefix}{Format(deviceId)}/down/#";

    public static string Down(Guid deviceId, string channel) => $"{Prefix}{Format(deviceId)}/down/{channel}";

    public static string Up(Guid deviceId, string channel) => $"{Prefix}{Format(deviceId)}/up/{channel}";

    public static string Format(Guid deviceId) => deviceId.ToString("D");

    public static bool TryParse(string topic, out Guid deviceId, out string direction, out string channel)
    {
        deviceId = Guid.Empty;
        direction = string.Empty;
        channel = string.Empty;
        if (!topic.StartsWith(Prefix, StringComparison.Ordinal))
        {
            return false;
        }

        var parts = topic[Prefix.Length..].Split('/');
        if (parts.Length != 3 || !Guid.TryParseExact(parts[0], "D", out deviceId) || parts[0].Any(char.IsUpper))
        {
            return false;
        }

        direction = parts[1];
        channel = parts[2];
        return direction is TopicDirections.Up or TopicDirections.Down;
    }

    /// <summary>Topic <c>down</c> de cada tipo de mensaje Cloud → device.</summary>
    public static string ChannelFor(string messageType) => messageType switch
    {
        MessageTypes.TaskDispatch or MessageTypes.TaskCancel or MessageTypes.TaskUpdateAck
            or MessageTypes.TaskApprovalDecision => TopicChannels.Tasks,
        MessageTypes.RequestAccepted or MessageTypes.RequestRejected or MessageTypes.TaskUpdate => TopicChannels.Notifications,
        MessageTypes.PresenceHelloAck => TopicChannels.Control,
        _ => throw new ArgumentOutOfRangeException(nameof(messageType), messageType, "Not a cloud-to-device message type."),
    };
}

public static class MessageTypes
{
    // down/tasks
    public const string TaskDispatch = "task.dispatch";
    public const string TaskCancel = "task.cancel";
    public const string TaskApprovalDecision = "task.approval_decision";
    public const string TaskUpdateAck = "task.update.ack";

    // down/notifications
    public const string RequestAccepted = "request.accepted";
    public const string RequestRejected = "request.rejected";
    public const string TaskUpdate = "task.update";

    // down/control
    public const string PresenceHelloAck = "presence.hello.ack";

    // up/tasks
    public const string TaskAck = "task.ack";
    public const string TaskRejected = "task.rejected";
    public const string TaskStarted = "task.started";
    public const string TaskProgress = "task.progress";
    public const string TaskApprovalRequired = "task.approval_required";
    public const string TaskCompleted = "task.completed";
    public const string TaskFailed = "task.failed";
    public const string TaskCancelled = "task.cancelled";

    // up/requests
    public const string TaskCreate = "task.create";
    public const string TaskGet = "task.get";
    public const string TaskCancelRequest = "task.cancel_request";

    // up/presence
    public const string PresenceHello = "presence.hello";
    public const string PresenceHeartbeat = "presence.heartbeat";
    public const string PresenceGoodbye = "presence.goodbye";
    public const string PresenceOffline = "presence.offline";

    // up/events (reservado)
    public const string DeviceEvent = "device.event";
}
