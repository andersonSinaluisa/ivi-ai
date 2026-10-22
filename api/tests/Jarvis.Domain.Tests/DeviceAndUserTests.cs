using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Users;

namespace Jarvis.Domain.Tests;

public sealed class DeviceTests
{
    private static readonly DateTimeOffset T0 = new(2026, 9, 19, 15, 0, 0, TimeSpan.Zero);

    private static Device NewDevice(string name = "PC Escritorio")
    {
        var id = Guid.CreateVersion7();
        return Device.Register(id, Guid.CreateVersion7(), name, DeviceType.Windows, DeviceCredential.CreateSecret(Guid.CreateVersion7(), id, new byte[32], "abcd", T0), T0);
    }

    [Theory]
    [InlineData(DeviceAdminState.Disabled, PresenceState.Online, false, DeviceStatus.Disabled)]
    [InlineData(DeviceAdminState.Revoked, PresenceState.Online, true, DeviceStatus.Disabled)]
    [InlineData(DeviceAdminState.Active, PresenceState.Offline, true, DeviceStatus.Offline)]
    [InlineData(DeviceAdminState.Active, PresenceState.Stale, true, DeviceStatus.Stale)]
    [InlineData(DeviceAdminState.Active, PresenceState.Online, true, DeviceStatus.Busy)]
    [InlineData(DeviceAdminState.Active, PresenceState.Online, false, DeviceStatus.Online)]
    [InlineData(DeviceAdminState.Disabled, PresenceState.Offline, false, DeviceStatus.Disabled)]
    [InlineData(DeviceAdminState.Active, PresenceState.Stale, false, DeviceStatus.Stale)]
    public void Status_precedence(DeviceAdminState admin, PresenceState presence, bool busy, DeviceStatus expected) =>
        Assert.Equal(expected, Device.ComputeStatus(admin, presence, busy));

    [Fact]
    public void Register_starts_active_offline_with_one_active_credential()
    {
        var device = NewDevice();
        Assert.Equal(DeviceAdminState.Active, device.AdminState);
        Assert.Equal(PresenceState.Offline, device.PresenceState);
        Assert.NotNull(device.ActiveCredential);
        Assert.Equal([EventTypes.DeviceRegistered], device.DomainEvents.Select(e => e.Type));
    }

    [Theory]
    [InlineData("")]
    [InlineData(" padded ")]
    [InlineData("12345678901234567890123456789012345678901234567890123456789012345")]
    public void Register_rejects_invalid_names(string name) => Assert.Throws<DomainException>(() => NewDevice(name));

    [Fact]
    public void Disable_is_idempotent_and_forces_offline()
    {
        var device = NewDevice();
        device.Connect("s-1", [], [], "{}", false, null, null, T0);
        device.ClearDomainEvents();
        Assert.True(device.Disable(T0));
        Assert.False(device.Disable(T0));
        Assert.Equal(PresenceState.Offline, device.PresenceState);
        Assert.Equal([EventTypes.DeviceDisabled, EventTypes.DeviceDisconnected], device.DomainEvents.Select(e => e.Type));
    }

    [Fact]
    public void Revoked_is_irreversible()
    {
        var device = NewDevice();
        Assert.True(device.Revoke("lost", 0, T0));
        Assert.False(device.Revoke("again", 0, T0));
        Assert.Null(device.ActiveCredential);
        var exception = Assert.Throws<DomainException>(() => device.Enable(T0));
        Assert.Equal(DomainErrorCodes.DeviceRevoked, exception.Code);
    }

    [Fact]
    public void Rotate_keeps_exactly_one_active_credential()
    {
        var device = NewDevice();
        var newCredential = DeviceCredential.CreateSecret(Guid.CreateVersion7(), device.Id, new byte[32], "wxyz", T0);
        device.RotateCredential(newCredential, T0);
        Assert.Single(device.Credentials, c => c.Status == CredentialStatus.Active);
        Assert.Equal("wxyz", device.ActiveCredential!.SecretHint);
    }

    [Fact]
    public void Connect_filters_invalid_capabilities_and_types()
    {
        var device = NewDevice();
        device.Connect("s-1", ["agent", "Bad Cap", "display"], ["OPEN_CHROME", "bad"], "{}", false, "1.0", "1.0", T0);
        Assert.Equal(["agent", "display"], device.Capabilities);
        Assert.Equal(["OPEN_CHROME"], device.SupportedTaskTypes);
        Assert.Equal(PresenceState.Online, device.PresenceState);
    }

    [Fact]
    public void Lwt_of_old_session_is_ignored()
    {
        var device = NewDevice();
        device.Connect("s-old", [], [], "{}", false, null, null, T0);
        device.Connect("s-new", [], [], "{}", false, null, null, T0.AddSeconds(1));
        Assert.False(device.Disconnect("s-old", "lwt", T0.AddSeconds(2)));
        Assert.Equal(PresenceState.Online, device.PresenceState);
        Assert.True(device.Disconnect("s-new", "lwt", T0.AddSeconds(3)));
        Assert.Equal(PresenceState.Offline, device.PresenceState);
    }

    [Fact]
    public void Presence_sweeps_stale_then_offline_and_recovers()
    {
        var device = NewDevice();
        device.Connect("s-1", [], [], "{}", false, null, null, T0);
        Assert.False(device.MarkStaleIfSilent(T0.AddSeconds(75), TimeSpan.FromSeconds(75)));
        Assert.True(device.MarkStaleIfSilent(T0.AddSeconds(76), TimeSpan.FromSeconds(75)));
        Assert.Equal(PresenceChange.RecoveredFromStale, device.Touch(null, T0.AddSeconds(77)));
        Assert.Equal(PresenceState.Online, device.PresenceState);
        device.ClearDomainEvents();
        Assert.True(device.MarkOfflineIfSilent(T0.AddSeconds(300), TimeSpan.FromSeconds(180)));
        Assert.Equal("heartbeat_timeout", device.DomainEvents.Single().Data!["reason"]);
        Assert.Equal(PresenceChange.RecoveredFromOffline, device.Heartbeat("s-1", true, T0.AddSeconds(301)));
        Assert.Equal(DeviceStatus.Busy, device.Status);
    }

    [Fact]
    public void Grant_requires_distinct_devices_of_same_owner()
    {
        var a = NewDevice("a");
        var b = NewDevice("b");
        Assert.Throws<DomainException>(() => DeviceTaskGrant.Create(Guid.NewGuid(), a, b, "OPEN_CHROME", Guid.NewGuid(), T0));
        var exception = Assert.Throws<DomainException>(() => DeviceTaskGrant.Create(Guid.NewGuid(), a, a, "OPEN_CHROME", Guid.NewGuid(), T0));
        Assert.Equal("SAME_DEVICE", exception.Code);
    }

    [Fact]
    public void Credential_usage_is_throttled_to_once_per_minute()
    {
        var credential = DeviceCredential.CreateSecret(Guid.NewGuid(), Guid.NewGuid(), new byte[32], "abcd", T0);
        Assert.True(credential.MarkUsed(T0));
        Assert.False(credential.MarkUsed(T0.AddSeconds(30)));
        Assert.True(credential.MarkUsed(T0.AddSeconds(61)));
    }
}

public sealed class UserTests
{
    private static readonly DateTimeOffset T0 = new(2026, 9, 19, 15, 0, 0, TimeSpan.Zero);

    [Fact]
    public void Five_failures_lock_the_account_for_15_minutes()
    {
        var user = User.Create(Guid.NewGuid(), "owner@example.com", "Owner", "hash", T0);
        for (var i = 0; i < 4; i++)
        {
            Assert.False(user.RegisterFailedLogin(T0));
        }

        Assert.True(user.RegisterFailedLogin(T0));
        Assert.True(user.IsLockedOut(T0.AddMinutes(14)));
        Assert.False(user.IsLockedOut(T0.AddMinutes(16)));
        Assert.Contains(user.DomainEvents, e => e.Type == EventTypes.UserLockedOut);
    }

    [Fact]
    public void Successful_login_resets_counter()
    {
        var user = User.Create(Guid.NewGuid(), "owner@example.com", "Owner", "hash", T0);
        user.RegisterFailedLogin(T0);
        user.RegisterSuccessfulLogin(T0, "iphash");
        Assert.Equal(0, user.FailedLoginCount);
        Assert.Equal(T0, user.LastLoginAt);
    }

    [Fact]
    public void Refresh_token_revoke_records_replacement_once()
    {
        var token = RefreshToken.Issue(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), new byte[32], T0, TimeSpan.FromDays(30));
        Assert.True(token.IsActive(T0));
        var replacement = Guid.NewGuid();
        token.Revoke(T0, replacement);
        token.Revoke(T0.AddHours(1), Guid.NewGuid());
        Assert.Equal(replacement, token.ReplacedById);
        Assert.Equal(T0, token.RevokedAt);
        Assert.False(token.IsActive(T0));
    }
}
