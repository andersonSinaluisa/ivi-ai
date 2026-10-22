using Jarvis.Contracts;
using Jarvis.Domain.Common;

namespace Jarvis.Application.Common;

/// <summary>Error esperado de un caso de uso; <see cref="Code"/> pertenece a <see cref="ErrorCodes"/>.</summary>
public sealed record AppError(string Code, string Message, IReadOnlyDictionary<string, string[]>? Errors = null)
{
    public static AppError Validation(string field, string message) =>
        new(ErrorCodes.ValidationFailed, "Validation failed.", new Dictionary<string, string[]> { [field] = [message] });

    public static AppError NotFound(string what = "Resource") => new(ErrorCodes.NotFound, $"{what} not found.");

    public static AppError Forbidden(string message = "Operation not allowed for this principal.") => new(ErrorCodes.Forbidden, message);

    public static AppError FromDomain(DomainException exception) => new(exception.Code, exception.Message);
}

public sealed class Result<T>
{
    private readonly T? _value;

    private Result(T value)
    {
        _value = value;
        IsSuccess = true;
    }

    private Result(AppError error)
    {
        Error = error;
    }

    public bool IsSuccess { get; }

    public AppError? Error { get; }

    public T Value => IsSuccess ? _value! : throw new InvalidOperationException($"Result is a failure: {Error!.Code}");

    public static implicit operator Result<T>(T value) => new(value);

    public static implicit operator Result<T>(AppError error) => new(error);
}

/// <summary>Resultado sin valor.</summary>
public readonly record struct Unit
{
    public static readonly Unit Value;
}
