# HU-051: imagen de producción de jarvis-api (docs/06-infrastructure/docker-strategy.md)
FROM mcr.microsoft.com/dotnet/sdk:10.0 AS build
WORKDIR /src
COPY global.json Directory.Build.props Directory.Packages.props ./
COPY src/Jarvis.Domain/Jarvis.Domain.csproj src/Jarvis.Domain/
COPY src/Jarvis.Contracts/Jarvis.Contracts.csproj src/Jarvis.Contracts/
COPY src/Jarvis.Application/Jarvis.Application.csproj src/Jarvis.Application/
COPY src/Jarvis.Infrastructure/Jarvis.Infrastructure.csproj src/Jarvis.Infrastructure/
COPY src/Jarvis.Api/Jarvis.Api.csproj src/Jarvis.Api/
RUN dotnet restore src/Jarvis.Api/Jarvis.Api.csproj
COPY src/ src/
RUN dotnet publish src/Jarvis.Api/Jarvis.Api.csproj -c Release -o /app --no-restore /p:UseAppHost=true

# Runtime chiseled: sin shell ni package manager, usuario no-root (UID 1654).
FROM mcr.microsoft.com/dotnet/aspnet:10.0-noble-chiseled AS runtime
WORKDIR /app
COPY --from=build /app .
ENV ASPNETCORE_URLS=http://+:8080;http://+:8081 \
    DOTNET_gcServer=0 \
    DOTNET_TieredPGO=1
USER $APP_UID
EXPOSE 8080 8081
HEALTHCHECK --interval=15s --timeout=5s --retries=3 --start-period=30s CMD ["/app/jarvis-api", "healthcheck"]
ENTRYPOINT ["/app/jarvis-api"]
