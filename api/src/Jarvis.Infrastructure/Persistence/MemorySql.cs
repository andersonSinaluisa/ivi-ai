using System.Data;
using System.Data.Common;
using Jarvis.Application.Abstractions;
using Jarvis.Domain.Jobs;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Storage;
using Npgsql;
using NpgsqlTypes;

namespace Jarvis.Infrastructure.Persistence;

/// <summary>
/// SQL específico del esquema memory sobre la conexión (y la transacción) del DbContext del scope.
/// La búsqueda vectorial usa pgvector si está instalado: <c>real[]</c> → <c>vector</c> con un cast, sin fijar la
/// dimensión en el esquema (ADR-014).
/// </summary>
public sealed class MemorySql(JarvisDbContext db) : IMemorySql
{
    private static bool? _vectorAvailable;

    /// <summary>Expresión indexada (GIN) para la búsqueda de texto; debe coincidir con la de la migración.</summary>
    public const string SearchDocument = "to_tsvector('simple', summary || ' ' || coalesce(content, ''))";

    public const string ExperienceDocument = "to_tsvector('simple', goal || ' ' || result)";

    public Task LockOwnerChangesAsync(Guid ownerId, CancellationToken cancellationToken)
    {
        var key = "sync:" + ownerId.ToString("D");
        return db.Database.ExecuteSqlAsync($"SELECT pg_advisory_xact_lock(hashtextextended({key}, 0))", cancellationToken);
    }

    public async Task<long> NextSeqAsync(CancellationToken cancellationToken) =>
        await ScalarAsync<long>("SELECT nextval('memory.change_seq')", [], cancellationToken);

    public async Task<long> CurrentSeqAsync(Guid ownerId, CancellationToken cancellationToken) =>
        await ScalarAsync<long>(
            """
            SELECT coalesce(max(seq), 0) FROM (
              SELECT max(seq) AS seq FROM memory.memories WHERE owner_id = @o
              UNION ALL SELECT max(seq) FROM memory.memory_relations WHERE owner_id = @o
              UNION ALL SELECT max(seq) FROM memory.memory_embeddings WHERE owner_id = @o
              UNION ALL SELECT max(seq) FROM memory.experiences WHERE owner_id = @o
              UNION ALL SELECT max(seq) FROM memory.skills WHERE owner_id = @o) s
            """,
            [new NpgsqlParameter("o", ownerId)],
            cancellationToken);

    public async Task<IReadOnlyList<ChangeRef>> ChangedSinceAsync(
        Guid ownerId,
        long since,
        int limit,
        IReadOnlyCollection<string> entities,
        IReadOnlyCollection<string> embeddingModels,
        CancellationToken cancellationToken)
    {
        const string sql = """
            SELECT entity, entity_id, model, seq FROM (
              SELECT 'memory' AS entity, id AS entity_id, NULL::text AS model, seq FROM memory.memories WHERE owner_id = @o AND seq > @s
              UNION ALL SELECT 'relation', id, NULL, seq FROM memory.memory_relations WHERE owner_id = @o AND seq > @s
              UNION ALL SELECT 'embedding', memory_id, model, seq FROM memory.memory_embeddings WHERE owner_id = @o AND seq > @s AND model = ANY(@models)
              UNION ALL SELECT 'experience', id, NULL, seq FROM memory.experiences WHERE owner_id = @o AND seq > @s
              UNION ALL SELECT 'skill', id, NULL, seq FROM memory.skills WHERE owner_id = @o AND seq > @s
            ) c
            WHERE entity = ANY(@entities)
            ORDER BY seq
            LIMIT @limit
            """;
        var rows = new List<ChangeRef>();
        await using var command = await CommandAsync(
            sql,
            [
                new NpgsqlParameter("o", ownerId),
                new NpgsqlParameter("s", since),
                new NpgsqlParameter("models", NpgsqlDbType.Array | NpgsqlDbType.Text) { Value = embeddingModels.ToArray() },
                new NpgsqlParameter("entities", NpgsqlDbType.Array | NpgsqlDbType.Text) { Value = entities.ToArray() },
                new NpgsqlParameter("limit", limit),
            ],
            cancellationToken);
        await using var reader = await command.ExecuteReaderAsync(cancellationToken);
        while (await reader.ReadAsync(cancellationToken))
        {
            rows.Add(new ChangeRef(reader.GetString(0), reader.GetGuid(1), reader.IsDBNull(2) ? null : reader.GetString(2), reader.GetInt64(3)));
        }

        return rows;
    }

    /// <summary>Solo pruebas: fuerza la búsqueda vectorial en la aplicación (false) o con pgvector (true).</summary>
    public static bool? VectorOverride { get; set; }

    public async Task<bool> VectorExtensionAvailableAsync(CancellationToken cancellationToken)
    {
        if (VectorOverride is { } forced)
        {
            return forced;
        }

        if (_vectorAvailable is { } cached)
        {
            return cached;
        }

        var available = await ScalarAsync<bool>("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector')", [], cancellationToken);
        _vectorAvailable = available;
        return available;
    }

    /// <summary>Olvida el resultado cacheado (pruebas y reinstalación de la extensión).</summary>
    public static void ResetVectorCache() => _vectorAvailable = null;

    public async Task<IReadOnlyList<ScoredId>> VectorSearchAsync(Guid ownerId, string model, float[] vector, int limit, CancellationToken cancellationToken)
    {
        const string sql = """
            SELECT e.memory_id, 1 - (e.vector::vector <=> @q::real[]::vector) AS score
            FROM memory.memory_embeddings e
            JOIN memory.memories m ON m.id = e.memory_id
            WHERE e.owner_id = @o AND e.model = @m AND e.dims = @d
              AND m.deleted_at IS NULL AND e.content_sha256 = m.content_sha256
            ORDER BY e.vector::vector <=> @q::real[]::vector
            LIMIT @k
            """;
        return await ScoredAsync(
            sql,
            [
                new NpgsqlParameter("o", ownerId),
                new NpgsqlParameter("m", model),
                new NpgsqlParameter("d", vector.Length),
                new NpgsqlParameter("q", NpgsqlDbType.Array | NpgsqlDbType.Real) { Value = vector },
                new NpgsqlParameter("k", limit),
            ],
            cancellationToken);
    }

    public Task<IReadOnlyList<ScoredId>> TextSearchAsync(Guid ownerId, IReadOnlyList<string> terms, int limit, CancellationToken cancellationToken) =>
        terms.Count == 0
            ? Task.FromResult<IReadOnlyList<ScoredId>>([])
            : ScoredAsync(
                $"""
                SELECT id, ts_rank({SearchDocument}, q) AS score
                FROM memory.memories, to_tsquery('simple', @tsq) q
                WHERE owner_id = @o AND deleted_at IS NULL AND {SearchDocument} @@ q
                ORDER BY score DESC
                LIMIT @k
                """,
                [new NpgsqlParameter("o", ownerId), new NpgsqlParameter("tsq", string.Join(" | ", terms)), new NpgsqlParameter("k", limit)],
                cancellationToken);

    public Task<IReadOnlyList<ScoredId>> ExperienceSearchAsync(Guid ownerId, IReadOnlyList<string> terms, int limit, CancellationToken cancellationToken) =>
        terms.Count == 0
            ? Task.FromResult<IReadOnlyList<ScoredId>>([])
            : ScoredAsync(
                $"""
                SELECT id, ts_rank({ExperienceDocument}, q) AS score
                FROM memory.experiences, to_tsquery('simple', @tsq) q
                WHERE owner_id = @o AND deleted_at IS NULL AND {ExperienceDocument} @@ q
                ORDER BY score DESC, occurrences DESC
                LIMIT @k
                """,
                [new NpgsqlParameter("o", ownerId), new NpgsqlParameter("tsq", string.Join(" | ", terms)), new NpgsqlParameter("k", limit)],
                cancellationToken);

    public Task<List<JobRun>> ClaimRunsAsync(DateTimeOffset now, int batchSize, CancellationToken cancellationToken) =>
        db.JobRuns.FromSql($"""
            SELECT r.* FROM memory.job_runs r
            WHERE (r.status = 'queued' AND r.scheduled_for <= {now})
               OR (r.status = 'running' AND r.lease_until < {now})
            ORDER BY r.scheduled_for
            LIMIT {batchSize}
            FOR UPDATE SKIP LOCKED
            """)
            .IgnoreQueryFilters()
            .ToListAsync(cancellationToken);

    public Task<List<Job>> ClaimDueJobsAsync(DateTimeOffset now, int batchSize, CancellationToken cancellationToken) =>
        db.Jobs.FromSql($"""
            SELECT j.* FROM memory.jobs j
            WHERE j.enabled AND j.next_run_at <= {now}
              AND j.approval_state IN ('not_required', 'approved')
              AND NOT EXISTS (SELECT 1 FROM memory.job_runs r WHERE r.job_id = j.id AND r.status IN ('queued', 'running'))
            ORDER BY j.next_run_at
            LIMIT {batchSize}
            FOR UPDATE OF j SKIP LOCKED
            """)
            .IgnoreQueryFilters()
            .ToListAsync(cancellationToken);

    private async Task<IReadOnlyList<ScoredId>> ScoredAsync(string sql, NpgsqlParameter[] parameters, CancellationToken cancellationToken)
    {
        var rows = new List<ScoredId>();
        await using var command = await CommandAsync(sql, parameters, cancellationToken);
        await using var reader = await command.ExecuteReaderAsync(cancellationToken);
        while (await reader.ReadAsync(cancellationToken))
        {
            rows.Add(new ScoredId(reader.GetGuid(0), Convert.ToDouble(reader.GetValue(1), System.Globalization.CultureInfo.InvariantCulture)));
        }

        return rows;
    }

    private async Task<T> ScalarAsync<T>(string sql, NpgsqlParameter[] parameters, CancellationToken cancellationToken)
    {
        await using var command = await CommandAsync(sql, parameters, cancellationToken);
        var value = await command.ExecuteScalarAsync(cancellationToken);
        return (T)Convert.ChangeType(value!, typeof(T), System.Globalization.CultureInfo.InvariantCulture);
    }

    private async Task<DbCommand> CommandAsync(string sql, NpgsqlParameter[] parameters, CancellationToken cancellationToken)
    {
        var connection = db.Database.GetDbConnection();
        if (connection.State != ConnectionState.Open)
        {
            await db.Database.OpenConnectionAsync(cancellationToken);
        }

        var command = connection.CreateCommand();
        command.CommandText = sql;
        command.Transaction = db.Database.CurrentTransaction?.GetDbTransaction();
        command.Parameters.AddRange(parameters);
        return command;
    }
}
