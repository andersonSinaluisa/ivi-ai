using Jarvis.Domain.Common;
using Jarvis.Domain.Events;

namespace Jarvis.Domain.Users;

public enum UserStatus
{
    Active,
    Disabled,
}

public sealed class User : Entity
{
    public const int MaxFailedLogins = 5;
    public static readonly TimeSpan LockoutDuration = TimeSpan.FromMinutes(15);

    private User()
    {
    }

    public string Email { get; private set; } = string.Empty;

    public string DisplayName { get; private set; } = string.Empty;

    public string PasswordHash { get; private set; } = string.Empty;

    public UserStatus Status { get; private set; }

    public int FailedLoginCount { get; private set; }

    public DateTimeOffset? LockedUntil { get; private set; }

    public DateTimeOffset? LastLoginAt { get; private set; }

    public DateTimeOffset CreatedAt { get; private set; }

    public DateTimeOffset UpdatedAt { get; private set; }

    public static User Create(Guid id, string email, string displayName, string passwordHash, DateTimeOffset now)
    {
        if (string.IsNullOrWhiteSpace(email) || email.Length > 254 || !email.Contains('@', StringComparison.Ordinal))
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Invalid email.");
        }

        if (string.IsNullOrWhiteSpace(displayName) || displayName.Length > 100)
        {
            throw new DomainException(DomainErrorCodes.ValidationFailed, "Display name must be 1-100 characters.");
        }

        var user = new User
        {
            Id = id,
            Email = email.Trim(),
            DisplayName = displayName.Trim(),
            PasswordHash = passwordHash,
            Status = UserStatus.Active,
            CreatedAt = now,
            UpdatedAt = now,
        };
        user.Raise(new DomainEvent(
            EventTypes.UserCreated,
            id,
            now,
            Data: new Dictionary<string, object?> { ["via"] = "bootstrap-owner" },
            Actor: Actor.System));
        return user;
    }

    public bool IsLockedOut(DateTimeOffset now) => LockedUntil is { } until && until > now;

    /// <summary>Registra un intento fallido; devuelve true si la cuenta quedó bloqueada en este intento.</summary>
    public bool RegisterFailedLogin(DateTimeOffset now)
    {
        FailedLoginCount++;
        UpdatedAt = now;
        Raise(new DomainEvent(
            EventTypes.UserLoginFailed,
            Id,
            now,
            Data: new Dictionary<string, object?> { ["reason"] = "bad_password" },
            Actor: Actor.System));

        if (FailedLoginCount < MaxFailedLogins)
        {
            return false;
        }

        LockedUntil = now + LockoutDuration;
        FailedLoginCount = 0;
        Raise(new DomainEvent(
            EventTypes.UserLockedOut,
            Id,
            now,
            Data: new Dictionary<string, object?> { ["until"] = LockedUntil },
            Actor: Actor.System));
        return true;
    }

    public void RegisterLockedAttempt(DateTimeOffset now) =>
        Raise(new DomainEvent(
            EventTypes.UserLoginFailed,
            Id,
            now,
            Data: new Dictionary<string, object?> { ["reason"] = "locked" },
            Actor: Actor.System));

    public void RegisterSuccessfulLogin(DateTimeOffset now, string? ipHash)
    {
        FailedLoginCount = 0;
        LockedUntil = null;
        LastLoginAt = now;
        UpdatedAt = now;
        Raise(new DomainEvent(
            EventTypes.UserLoginSucceeded,
            Id,
            now,
            Data: new Dictionary<string, object?> { ["ip"] = ipHash },
            Actor: Actor.User(Id)));
    }

    /// <summary>
    /// Restablece la contraseña (solo desde la consola del servidor): quita el bloqueo por intentos fallidos. Las
    /// sesiones abiertas las revoca el handler.
    /// </summary>
    public void ResetPassword(string passwordHash, DateTimeOffset now)
    {
        PasswordHash = passwordHash;
        FailedLoginCount = 0;
        LockedUntil = null;
        UpdatedAt = now;
        Raise(new DomainEvent(
            EventTypes.UserPasswordReset,
            Id,
            now,
            Data: new Dictionary<string, object?> { ["via"] = "reset-owner-password" },
            Actor: Actor.System));
    }

    public void Disable(DateTimeOffset now)
    {
        Status = UserStatus.Disabled;
        UpdatedAt = now;
    }
}

public sealed class RefreshToken
{
    private RefreshToken()
    {
    }

    public Guid Id { get; private set; }

    public Guid UserId { get; private set; }

    public Guid FamilyId { get; private set; }

    public byte[] TokenHash { get; private set; } = [];

    public DateTimeOffset CreatedAt { get; private set; }

    public DateTimeOffset ExpiresAt { get; private set; }

    public DateTimeOffset? RevokedAt { get; private set; }

    public Guid? ReplacedById { get; private set; }

    public static RefreshToken Issue(Guid id, Guid userId, Guid familyId, byte[] tokenHash, DateTimeOffset now, TimeSpan lifetime) =>
        new()
        {
            Id = id,
            UserId = userId,
            FamilyId = familyId,
            TokenHash = tokenHash,
            CreatedAt = now,
            ExpiresAt = now + lifetime,
        };

    public bool IsActive(DateTimeOffset now) => RevokedAt is null && ExpiresAt > now;

    public void Revoke(DateTimeOffset now, Guid? replacedById = null)
    {
        RevokedAt ??= now;
        ReplacedById ??= replacedById;
    }
}
