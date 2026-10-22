# ERD

Detalle de columnas, constraints e índices: [Database Design](database-design.md).

```mermaid
erDiagram
    users ||--o{ refresh_tokens : "tiene"
    users ||--o{ devices : "owns"
    users ||--o{ tasks : "owns"
    users ||--o{ device_task_grants : "owns"
    users ||--o{ events : "owns"
    devices ||--o{ device_credentials : "tiene"
    devices ||--o{ tasks : "target"
    devices |o--o{ tasks : "source"
    devices ||--o{ device_task_grants : "source"
    devices ||--o{ device_task_grants : "target"

    users {
        uuid id PK
        citext email UK
        text display_name
        text password_hash
        text status
        int failed_login_count
        timestamptz locked_until
        timestamptz last_login_at
        timestamptz created_at
        timestamptz updated_at
    }
    refresh_tokens {
        uuid id PK
        uuid user_id FK
        uuid family_id
        bytea token_hash UK
        timestamptz expires_at
        timestamptz revoked_at
        uuid replaced_by_id FK
    }
    devices {
        uuid id PK
        uuid owner_id FK
        text name
        text type
        text admin_state
        text presence_state
        boolean reported_busy
        text current_session_id
        text_array capabilities
        text_array supported_task_types
        jsonb reported_info
        timestamptz last_seen_at
        timestamptz broker_synced_at
        timestamptz created_at
    }
    device_credentials {
        uuid id PK
        uuid device_id FK
        text kind
        bytea secret_hash
        text secret_hint
        text certificate_thumbprint
        text status
        timestamptz expires_at
        timestamptz revoked_at
        timestamptz last_used_at
    }
    device_task_grants {
        uuid id PK
        uuid owner_id FK
        uuid source_device_id FK
        uuid target_device_id FK
        text task_type
        uuid created_by_user_id FK
    }
    tasks {
        uuid id PK
        uuid owner_id FK
        text requested_by_type
        uuid requested_by_user_id FK
        uuid source_device_id FK
        uuid target_device_id FK
        text type
        jsonb payload
        text status
        uuid correlation_id
        text idempotency_key
        int attempt_count
        timestamptz expires_at
        timestamptz dispatched_at
        timestamptz delivered_at
        timestamptz started_at
        timestamptz completed_at
        jsonb result
        text error_code
    }
    events {
        bigint seq PK
        uuid id UK
        uuid owner_id FK
        text type
        timestamptz occurred_at
        uuid correlation_id
        text actor_type
        uuid actor_id
        uuid device_id
        uuid task_id
        jsonb data
    }
```

`events.device_id` y `events.task_id` no tienen FK a propósito (ver [Database Design](database-design.md#events)); el diagrama no dibuja esas relaciones.
