-- Aprovisionamiento idempotente de Jarvis en un PostgreSQL NATIVO (>= 15) (docs/04-data/database-design.md#roles-de-base-de-datos).
-- Lo ejecuta deploy.sh como el superusuario local:  sudo -u postgres psql -v ON_ERROR_STOP=1 -f provision.sql
-- Variables psql requeridas (deploy.sh las define por stdin con \set, nunca como argumentos):
--   db_name, migrator_pw, app_pw, maintenance_pw, backup_pw, worker_pw
-- Seguro de re-ejecutar: crea lo que falta y resincroniza contraseñas con los archivos de secretos.

\set ON_ERROR_STOP on

-- Roles de mínimo privilegio
SELECT 'CREATE ROLE jarvis_migrator LOGIN' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_migrator') \gexec
SELECT 'CREATE ROLE jarvis_app LOGIN' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_app') \gexec
SELECT 'CREATE ROLE jarvis_maintenance LOGIN' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_maintenance') \gexec
SELECT 'CREATE ROLE jarvis_backup LOGIN' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_backup') \gexec
-- jarvis-worker (ADR-016): solo lo que necesitan los jobs del catálogo; los GRANT los aplica la migración.
SELECT 'CREATE ROLE jarvis_worker LOGIN' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'jarvis_worker') \gexec

ALTER ROLE jarvis_migrator WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD :'migrator_pw';
ALTER ROLE jarvis_app WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD :'app_pw';
ALTER ROLE jarvis_maintenance WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD :'maintenance_pw';
ALTER ROLE jarvis_backup WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD :'backup_pw';
ALTER ROLE jarvis_worker WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD :'worker_pw';
GRANT pg_read_all_data TO jarvis_backup;

-- Base de datos (propiedad de postgres; el esquema de la aplicación es de jarvis_migrator)
SELECT format('CREATE DATABASE %I', :'db_name') WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = :'db_name') \gexec
SELECT format('REVOKE ALL ON DATABASE %I FROM PUBLIC', :'db_name') \gexec
SELECT format('GRANT CONNECT ON DATABASE %I TO jarvis_migrator, jarvis_app, jarvis_maintenance, jarvis_backup, jarvis_worker', :'db_name') \gexec
-- CREATE SCHEMA IF NOT EXISTS (emitido por las migraciones) exige CREATE sobre la base aunque el esquema exista.
SELECT format('GRANT CREATE ON DATABASE %I TO jarvis_migrator', :'db_name') \gexec

\connect :db_name

-- citext requiere superusuario: se crea aquí; la migración solo hace CREATE EXTENSION IF NOT EXISTS.
CREATE EXTENSION IF NOT EXISTS citext;
CREATE SCHEMA IF NOT EXISTS jarvis AUTHORIZATION jarvis_migrator;
ALTER SCHEMA jarvis OWNER TO jarvis_migrator;
-- Memoria, experiencias, habilidades, sync y jobs (ADR-013): esquema propio, también de jarvis_migrator.
CREATE SCHEMA IF NOT EXISTS memory AUTHORIZATION jarvis_migrator;
ALTER SCHEMA memory OWNER TO jarvis_migrator;
-- pgvector (ADR-014) es OPCIONAL: si el paquete del sistema está instalado se activa SOLO en esta base; si no, la
-- búsqueda por similitud se hace en la aplicación. Nunca se instalan paquetes desde aquí.
SELECT 'CREATE EXTENSION IF NOT EXISTS vector' WHERE EXISTS (SELECT FROM pg_available_extensions WHERE name = 'vector') \gexec
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
-- Los GRANT sobre tablas los aplica la migración InitialSchema (ejecutada por jarvis_migrator).
