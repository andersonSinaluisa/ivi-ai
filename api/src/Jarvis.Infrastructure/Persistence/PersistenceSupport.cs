using System.Text.RegularExpressions;
using Jarvis.Application.Abstractions;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Design;
using Npgsql;

namespace Jarvis.Infrastructure.Persistence;

/// <summary>Usado por <c>dotnet ef</c> para generar migraciones sin levantar la API.</summary>
public sealed class DesignTimeDbContextFactory : IDesignTimeDbContextFactory<JarvisDbContext>
{
    public JarvisDbContext CreateDbContext(string[] args)
    {
        var options = new DbContextOptionsBuilder<JarvisDbContext>();
        PostgresSetup.Configure(options, Environment.GetEnvironmentVariable("JARVIS_DESIGN_CONNECTION")
            ?? "Host=localhost;Database=jarvis;Username=postgres;Password=postgres");
        return new JarvisDbContext(options.Options);
    }
}

public static partial class PostgresSetup
{
    public const string MigrationsHistoryTable = "__ef_migrations_history";

    public static void Configure(DbContextOptionsBuilder builder, string connectionString) =>
        builder.UseNpgsql(
            connectionString,
            npgsql => npgsql.MigrationsHistoryTable(MigrationsHistoryTable, JarvisDbContext.Schema));

    /// <summary>Sustituye <c>Password_File=/ruta</c> por <c>Password=contenido</c> (Docker secrets).</summary>
    public static string ResolvePasswordFile(string connectionString)
    {
        var match = PasswordFileRegex().Match(connectionString);
        if (!match.Success)
        {
            return connectionString;
        }

        var path = match.Groups["path"].Value.Trim();
        var password = File.ReadAllText(path).Trim();
        var builder = new NpgsqlConnectionStringBuilder(PasswordFileRegex().Replace(connectionString, string.Empty)) { Password = password };
        return builder.ConnectionString;
    }

    [GeneratedRegex(@"Password_File\s*=\s*(?<path>[^;]+);?", RegexOptions.IgnoreCase)]
    private static partial Regex PasswordFileRegex();
}

/// <summary>HU-040: borrado por lotes con el rol <c>jarvis_maintenance</c>.</summary>
public sealed class RetentionStore(string connectionString) : IRetentionStore
{
    public Task<int> DeleteEventsRecordedBeforeAsync(DateTimeOffset cutoff, int batchSize, CancellationToken cancellationToken) =>
        ExecuteAsync(
            "DELETE FROM jarvis.events WHERE seq IN (SELECT seq FROM jarvis.events WHERE recorded_at < @cutoff ORDER BY seq LIMIT @batch)",
            cutoff,
            batchSize,
            cancellationToken);

    public Task<int> DeleteRefreshTokensExpiredBeforeAsync(DateTimeOffset cutoff, int batchSize, CancellationToken cancellationToken) =>
        ExecuteAsync(
            "DELETE FROM jarvis.refresh_tokens WHERE id IN (SELECT id FROM jarvis.refresh_tokens WHERE expires_at < @cutoff OR revoked_at < @cutoff LIMIT @batch)",
            cutoff,
            batchSize,
            cancellationToken);

    private async Task<int> ExecuteAsync(string sql, DateTimeOffset cutoff, int batchSize, CancellationToken cancellationToken)
    {
        await using var connection = new NpgsqlConnection(connectionString);
        await connection.OpenAsync(cancellationToken);
        await using var command = new NpgsqlCommand(sql, connection);
        command.Parameters.AddWithValue("cutoff", cutoff);
        command.Parameters.AddWithValue("batch", batchSize);
        return await command.ExecuteNonQueryAsync(cancellationToken);
    }
}
