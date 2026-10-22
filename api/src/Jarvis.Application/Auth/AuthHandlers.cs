using System.Reflection;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Contracts;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Events;
using Jarvis.Domain.Users;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace Jarvis.Application.Auth;

public static class PasswordPolicy
{
    public const int MinLength = 12;

    private static readonly Lazy<HashSet<string>> CommonPasswords = new(LoadCommonPasswords);

    public static string? Validate(string? password)
    {
        if (string.IsNullOrEmpty(password) || password.Length < MinLength)
        {
            return $"Password must have at least {MinLength} characters.";
        }

        if (password.Length > 256)
        {
            return "Password is too long.";
        }

        return CommonPasswords.Value.Contains(password.ToLowerInvariant()) ? "Password is too common." : null;
    }

    private static HashSet<string> LoadCommonPasswords()
    {
        var assembly = Assembly.GetExecutingAssembly();
        var name = assembly.GetManifestResourceNames().Single(n => n.EndsWith("common-passwords.txt", StringComparison.Ordinal));
        using var stream = assembly.GetManifestResourceStream(name)!;
        using var reader = new StreamReader(stream);
        return reader.ReadToEnd()
            .Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .ToHashSet(StringComparer.Ordinal);
    }
}

/// <summary>HU-008: crea owners desde la línea de comandos (no hay registro público).</summary>
public sealed class BootstrapOwnerHandler(IJarvisDbContext db, IUserPasswordHasher hasher, TimeProvider clock, IRequestContext context)
{
    public async Task<Result<Guid>> HandleAsync(string email, string displayName, string password, CancellationToken cancellationToken)
    {
        var policyError = PasswordPolicy.Validate(password);
        if (policyError is not null)
        {
            return AppError.Validation("password", policyError);
        }

        context.EnterSystemScope();
        var normalized = email.Trim();
        if (await db.Users.AnyAsync(u => u.Email == normalized, cancellationToken))
        {
            return new AppError(ErrorCodes.ValidationFailed, "A user with this email already exists.");
        }

        try
        {
            var user = User.Create(Guid.CreateVersion7(), normalized, displayName, hasher.Hash(password), clock.GetUtcNow());
            db.Users.Add(user);
            await db.SaveChangesAsync(cancellationToken);
            return user.Id;
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }
    }
}

/// <summary>
/// Restablece la contraseña de un usuario existente desde la consola del servidor (quien tiene acceso root al VPS).
/// Cierra todas sus sesiones (refresh tokens revocados); los access tokens caducan solos en minutos.
/// </summary>
public sealed class ResetOwnerPasswordHandler(IJarvisDbContext db, IUserPasswordHasher hasher, TimeProvider clock, IRequestContext context)
{
    public async Task<Result<Guid>> HandleAsync(string email, string password, CancellationToken cancellationToken)
    {
        var policyError = PasswordPolicy.Validate(password);
        if (policyError is not null)
        {
            return AppError.Validation("password", policyError);
        }

        context.EnterSystemScope();
        var normalized = email.Trim();
        var user = await db.Users.FirstOrDefaultAsync(u => u.Email == normalized, cancellationToken);
        if (user is null)
        {
            return new AppError(ErrorCodes.ValidationFailed, "No user with this email.");
        }

        var now = clock.GetUtcNow();
        user.ResetPassword(hasher.Hash(password), now);
        var sessions = await db.RefreshTokens.Where(t => t.UserId == user.Id && t.RevokedAt == null).ToListAsync(cancellationToken);
        foreach (var session in sessions)
        {
            session.Revoke(now);
        }

        await db.SaveChangesAsync(cancellationToken);
        return user.Id;
    }
}

public sealed class LoginHandler(
    IJarvisDbContext db,
    IUserPasswordHasher hasher,
    ITokenIssuer tokens,
    IIpHasher ipHasher,
    TimeProvider clock,
    IOptions<JwtOptions> jwtOptions,
    ILogger<LoginHandler> logger)
{
    public async Task<Result<LoginResponse>> HandleAsync(string? email, string? password, string? ipAddress, CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(email) || string.IsNullOrEmpty(password))
        {
            return AppError.Validation("email", "Email and password are required.");
        }

        var now = clock.GetUtcNow();
        var normalized = email.Trim();
        var user = await db.Users.FirstOrDefaultAsync(u => u.Email == normalized, cancellationToken);
        if (user is null)
        {
            hasher.VerifyAgainstDummy(password);
            db.AddEvent(new DomainEvent(
                EventTypes.UserLoginFailed,
                null,
                now,
                Data: new Dictionary<string, object?> { ["reason"] = "unknown_user" },
                Actor: Actor.System));
            await db.SaveChangesAsync(cancellationToken);
            return InvalidCredentials();
        }

        if (user.IsLockedOut(now))
        {
            user.RegisterLockedAttempt(now);
            await db.SaveChangesAsync(cancellationToken);
            return new AppError(ErrorCodes.AccountLocked, "Account temporarily locked.");
        }

        if (!hasher.Verify(user.PasswordHash, password) || user.Status != UserStatus.Active)
        {
            if (user.RegisterFailedLogin(now))
            {
                logger.LogWarning("User {UserId} locked out after repeated failed logins", user.Id);
            }

            await db.SaveChangesAsync(cancellationToken);
            return InvalidCredentials();
        }

        user.RegisterSuccessfulLogin(now, ipHasher.Hash(ipAddress));
        var response = IssueSession(user, Guid.CreateVersion7(), now);
        await db.SaveChangesAsync(cancellationToken);
        logger.LogInformation("User {UserId} logged in", user.Id);
        return response;
    }

    internal LoginResponse IssueSession(User user, Guid familyId, DateTimeOffset now, RefreshToken? replaced = null)
    {
        var refreshValue = SecretGenerator.NewRefreshToken();
        var refresh = RefreshToken.Issue(
            Guid.CreateVersion7(),
            user.Id,
            familyId,
            SecretGenerator.Sha256(refreshValue),
            now,
            TimeSpan.FromDays(jwtOptions.Value.RefreshTokenDays));
        replaced?.Revoke(now, refresh.Id);
        db.RefreshTokens.Add(refresh);
        var access = tokens.IssueForUser(user);
        return new LoginResponse(
            access.Token,
            access.ExpiresAt,
            refreshValue,
            refresh.ExpiresAt,
            new UserSummaryDto(user.Id, user.Email, user.DisplayName));
    }

    private static AppError InvalidCredentials() => new(ErrorCodes.InvalidCredentials, "Invalid email or password.");
}

public sealed class RefreshHandler(IJarvisDbContext db, LoginHandler login, TimeProvider clock, ILogger<RefreshHandler> logger)
{
    public async Task<Result<LoginResponse>> HandleAsync(string? refreshToken, CancellationToken cancellationToken)
    {
        if (string.IsNullOrEmpty(refreshToken) || !refreshToken.StartsWith(SecretGenerator.RefreshTokenPrefix, StringComparison.Ordinal))
        {
            return Invalid();
        }

        var now = clock.GetUtcNow();
        var hash = SecretGenerator.Sha256(refreshToken);
        var token = await db.RefreshTokens.FirstOrDefaultAsync(t => t.TokenHash == hash, cancellationToken);
        if (token is null)
        {
            return Invalid();
        }

        if (token.RevokedAt is not null)
        {
            if (token.ReplacedById is not null)
            {
                // Reutilización de un token ya rotado: posible robo. Se revoca toda la familia.
                var family = await db.RefreshTokens
                    .Where(t => t.FamilyId == token.FamilyId && t.RevokedAt == null)
                    .ToListAsync(cancellationToken);
                family.ForEach(t => t.Revoke(now));
                await db.SaveChangesAsync(cancellationToken);
                logger.LogWarning("Refresh token reuse detected for user {UserId}; family revoked", token.UserId);
            }

            return Invalid();
        }

        if (!token.IsActive(now))
        {
            return Invalid();
        }

        var user = await db.Users.FirstOrDefaultAsync(u => u.Id == token.UserId, cancellationToken);
        if (user is null || user.Status != UserStatus.Active)
        {
            return Invalid();
        }

        var response = login.IssueSession(user, token.FamilyId, now, token);
        await db.SaveChangesAsync(cancellationToken);
        return response;
    }

    private static AppError Invalid() => new(ErrorCodes.InvalidRefreshToken, "Invalid refresh token.");
}

public sealed class LogoutHandler(IJarvisDbContext db, TimeProvider clock)
{
    public async Task HandleAsync(Guid userId, string? refreshToken, CancellationToken cancellationToken)
    {
        if (string.IsNullOrEmpty(refreshToken))
        {
            return;
        }

        var hash = SecretGenerator.Sha256(refreshToken);
        var token = await db.RefreshTokens.FirstOrDefaultAsync(t => t.TokenHash == hash && t.UserId == userId, cancellationToken);
        if (token is { RevokedAt: null })
        {
            token.Revoke(clock.GetUtcNow());
            await db.SaveChangesAsync(cancellationToken);
        }
    }
}

/// <summary>HU-017: intercambia la credencial de un device por un JWT de 10 min.</summary>
public sealed class DeviceTokenHandler(
    IJarvisDbContext db,
    IDeviceSecretHasher hasher,
    ITokenIssuer tokens,
    IRequestContext context,
    TimeProvider clock)
{
    private static readonly byte[] DummyHash = new byte[32];

    public async Task<Result<AccessTokenResponse>> HandleAsync(Guid? deviceId, string? secret, CancellationToken cancellationToken)
    {
        if (deviceId is null || string.IsNullOrEmpty(secret))
        {
            return AppError.Validation("deviceId", "deviceId and secret are required.");
        }

        context.EnterSystemScope();
        var now = clock.GetUtcNow();
        var device = await db.Devices.Include(d => d.Credentials).FirstOrDefaultAsync(d => d.Id == deviceId, cancellationToken);
        var credential = device?.ActiveCredential;
        var valid = credential?.SecretHash is { } expected && credential.IsUsable(now)
            ? hasher.Verify(secret, expected)
            : hasher.Verify(secret, DummyHash) && false;

        if (device is null || !valid || device.AdminState == DeviceAdminState.Revoked)
        {
            db.AddEvent(new DomainEvent(
                EventTypes.DeviceAuthFailed,
                device?.OwnerId,
                now,
                DeviceId: device?.Id,
                Data: new Dictionary<string, object?> { ["deviceId"] = deviceId, ["channel"] = "rest" },
                Actor: Actor.System));
            await db.SaveChangesAsync(cancellationToken);
            return new AppError(ErrorCodes.InvalidCredentials, "Invalid device credentials.");
        }

        if (device.AdminState == DeviceAdminState.Disabled)
        {
            return new AppError(ErrorCodes.DeviceDisabled, "Device is disabled.");
        }

        if (credential!.MarkUsed(now))
        {
            await db.SaveChangesAsync(cancellationToken);
        }

        var token = tokens.IssueForDevice(device);
        return new AccessTokenResponse(token.Token, token.ExpiresAt);
    }
}

public sealed class MeHandler(IJarvisDbContext db)
{
    public async Task<Result<MeResponse>> HandleAsync(Principal principal, CancellationToken cancellationToken)
    {
        if (principal.IsUser)
        {
            var user = await db.Users.AsNoTracking().FirstOrDefaultAsync(u => u.Id == principal.Id, cancellationToken);
            return user is null
                ? AppError.NotFound("User")
                : new MeResponse("User", user.Id, user.Id, Email: user.Email, DisplayName: user.DisplayName);
        }

        var device = await db.Devices.AsNoTracking().FirstOrDefaultAsync(d => d.Id == principal.Id, cancellationToken);
        return device is null
            ? AppError.NotFound("Device")
            : new MeResponse("Device", device.Id, device.OwnerId, Name: device.Name, Type: device.Type.ToString());
    }
}
