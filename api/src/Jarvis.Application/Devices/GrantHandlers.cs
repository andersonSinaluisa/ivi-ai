using Jarvis.Application.Abstractions;
using Jarvis.Application.Common;
using Jarvis.Contracts;
using Jarvis.Contracts.Rest;
using Jarvis.Domain.Common;
using Jarvis.Domain.Devices;
using Microsoft.EntityFrameworkCore;

namespace Jarvis.Application.Devices;

/// <summary>HU-025: grants (source, target, taskType) administrados por el owner.</summary>
public sealed class GrantHandler(IJarvisDbContext db, TimeProvider clock)
{
    public async Task<Result<IReadOnlyList<GrantDto>>> ListAsync(Principal principal, Guid sourceDeviceId, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        if (await db.FindOwnedAsync(principal, sourceDeviceId, cancellationToken, includeCredentials: false) is null)
        {
            return AppError.NotFound("Device");
        }

        var grants = await db.DeviceTaskGrants.AsNoTracking()
            .Where(g => g.SourceDeviceId == sourceDeviceId && g.OwnerId == principal.OwnerId)
            .OrderBy(g => g.CreatedAt)
            .ToListAsync(cancellationToken);
        return grants.Select(ToDto).ToList();
    }

    public async Task<Result<GrantDto>> CreateAsync(Principal principal, Guid sourceDeviceId, CreateGrantRequest request, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        if (request.TaskType == "*" || !DomainRules.IsValidTaskType(request.TaskType))
        {
            return AppError.Validation("taskType", "taskType must match ^[A-Z][A-Z0-9_]{1,63}$ (wildcards are not allowed).");
        }

        if (request.TargetDeviceId is null)
        {
            return AppError.Validation("targetDeviceId", "targetDeviceId is required.");
        }

        var source = await db.FindOwnedAsync(principal, sourceDeviceId, cancellationToken, includeCredentials: false);
        var target = await db.FindOwnedAsync(principal, request.TargetDeviceId.Value, cancellationToken, includeCredentials: false);
        if (source is null || target is null)
        {
            return AppError.NotFound("Device");
        }

        if (await db.DeviceTaskGrants.AnyAsync(
                g => g.SourceDeviceId == source.Id && g.TargetDeviceId == target.Id && g.TaskType == request.TaskType,
                cancellationToken))
        {
            return AlreadyExists();
        }

        try
        {
            var grant = DeviceTaskGrant.Create(Guid.CreateVersion7(), source, target, request.TaskType!, principal.Id, clock.GetUtcNow());
            db.DeviceTaskGrants.Add(grant);
            await db.SaveChangesAsync(cancellationToken);
            return ToDto(grant);
        }
        catch (DomainException exception)
        {
            return AppError.FromDomain(exception);
        }
        catch (Exception exception) when (db.IsUniqueViolation(exception))
        {
            db.ResetTracking();
            return AlreadyExists();
        }
    }

    public async Task<Result<Unit>> DeleteAsync(Principal principal, Guid sourceDeviceId, Guid grantId, CancellationToken cancellationToken)
    {
        if (!principal.IsUser)
        {
            return AppError.Forbidden();
        }

        if (await db.FindOwnedAsync(principal, sourceDeviceId, cancellationToken, includeCredentials: false) is null)
        {
            return AppError.NotFound("Device");
        }

        var grant = await db.DeviceTaskGrants.FirstOrDefaultAsync(
            g => g.Id == grantId && g.SourceDeviceId == sourceDeviceId && g.OwnerId == principal.OwnerId,
            cancellationToken);
        if (grant is not null)
        {
            grant.MarkDeleted(clock.GetUtcNow());
            db.DeviceTaskGrants.Remove(grant);
            await db.SaveChangesAsync(cancellationToken);
        }

        return Unit.Value;
    }

    private static GrantDto ToDto(DeviceTaskGrant grant) =>
        new(grant.Id, grant.SourceDeviceId, grant.TargetDeviceId, grant.TaskType, grant.CreatedAt);

    private static AppError AlreadyExists() => new(ErrorCodes.GrantAlreadyExists, "The grant already exists.");
}
