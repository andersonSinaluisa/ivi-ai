using System.Security.Claims;
using System.Security.Cryptography;
using System.Text;
using Jarvis.Application.Abstractions;
using Jarvis.Application.Configuration;
using Jarvis.Domain.Devices;
using Jarvis.Domain.Users;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Options;
using Microsoft.IdentityModel.JsonWebTokens;
using Microsoft.IdentityModel.Tokens;

namespace Jarvis.Infrastructure.Security;

public static class JarvisClaims
{
    public const string Type = "typ";
    public const string OwnerId = "owner_id";
    public const string UserType = "user";
    public const string DeviceType = "device";
}

/// <summary><c>HMAC-SHA256(pepper, secret)</c>: el secreto tiene 256 bits de entropía, un hash lento no aporta.</summary>
public sealed class HmacDeviceSecretHasher(IOptions<JarvisOptions> options) : IDeviceSecretHasher
{
    private readonly byte[] _pepper = Encoding.UTF8.GetBytes(options.Value.DeviceSecretPepper);

    public byte[] Hash(string secret) => HMACSHA256.HashData(_pepper, Encoding.UTF8.GetBytes(secret));

    public bool Verify(string secret, byte[] expectedHash) => CryptographicOperations.FixedTimeEquals(Hash(secret), expectedHash);
}

/// <summary>PasswordHasher de ASP.NET Core Identity (PBKDF2-HMAC-SHA512, 100 000 iteraciones).</summary>
public sealed class IdentityUserPasswordHasher : IUserPasswordHasher
{
    private readonly PasswordHasher<User> _hasher = new();
    private readonly Lazy<string> _dummyHash;

    public IdentityUserPasswordHasher() => _dummyHash = new Lazy<string>(() => _hasher.HashPassword(null!, Convert.ToBase64String(RandomNumberGenerator.GetBytes(24))));

    public string Hash(string password) => _hasher.HashPassword(null!, password);

    public bool Verify(string passwordHash, string providedPassword) =>
        _hasher.VerifyHashedPassword(null!, passwordHash, providedPassword) != PasswordVerificationResult.Failed;

    public void VerifyAgainstDummy(string providedPassword) => Verify(_dummyHash.Value, providedPassword);
}

public sealed class JwtTokenIssuer(IOptions<JwtOptions> options, TimeProvider clock) : ITokenIssuer
{
    private readonly JsonWebTokenHandler _handler = new();

    public IssuedToken IssueForUser(User user) =>
        Issue(user.Id, user.Id, JarvisClaims.UserType, TimeSpan.FromMinutes(options.Value.UserAccessTokenMinutes));

    public IssuedToken IssueForDevice(Device device) =>
        Issue(device.Id, device.OwnerId, JarvisClaims.DeviceType, TimeSpan.FromMinutes(options.Value.DeviceAccessTokenMinutes));

    public static SymmetricSecurityKey CreateKey(string secret, string keyId) =>
        new(Encoding.UTF8.GetBytes(secret)) { KeyId = keyId };

    private IssuedToken Issue(Guid subject, Guid ownerId, string type, TimeSpan lifetime)
    {
        var now = clock.GetUtcNow();
        var expires = now + lifetime;
        var descriptor = new SecurityTokenDescriptor
        {
            Issuer = options.Value.Issuer,
            Audience = options.Value.Audience,
            IssuedAt = now.UtcDateTime,
            NotBefore = now.UtcDateTime,
            Expires = expires.UtcDateTime,
            SigningCredentials = new SigningCredentials(CreateKey(options.Value.SigningKey, "current"), SecurityAlgorithms.HmacSha256),
            Subject = new ClaimsIdentity(
            [
                new Claim(JwtRegisteredClaimNames.Sub, subject.ToString("D")),
                new Claim(JarvisClaims.Type, type),
                new Claim(JarvisClaims.OwnerId, ownerId.ToString("D")),
                new Claim(JwtRegisteredClaimNames.Jti, Guid.NewGuid().ToString("N")),
            ]),
        };
        return new IssuedToken(_handler.CreateToken(descriptor), expires);
    }
}

/// <summary>HMAC de la IP con una clave derivada por día (HKDF sobre la clave JWT): correlacionable, no reversible.</summary>
public sealed class DailyIpHasher(IOptions<JwtOptions> options, TimeProvider clock) : IIpHasher
{
    public string? Hash(string? ipAddress)
    {
        if (string.IsNullOrEmpty(ipAddress))
        {
            return null;
        }

        var day = clock.GetUtcNow().ToString("yyyy-MM-dd", System.Globalization.CultureInfo.InvariantCulture);
        var key = HKDF.DeriveKey(HashAlgorithmName.SHA256, Encoding.UTF8.GetBytes(options.Value.SigningKey), 32, info: Encoding.UTF8.GetBytes("ip-hash:" + day));
        return Convert.ToHexStringLower(HMACSHA256.HashData(key, Encoding.UTF8.GetBytes(ipAddress)))[..16];
    }
}
