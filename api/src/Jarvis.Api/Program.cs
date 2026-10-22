using Jarvis.Api;
using Jarvis.Api.Commands;

// Un solo binario: servidor (sin argumentos) o subcomandos de administración (docs/06-infrastructure/docker-strategy.md).
var command = args.Length > 0 && !args[0].StartsWith('-') ? args[0] : null;
var rest = command is null ? args : args[1..];

return command switch
{
    null => await RunServerAsync(rest),
    "migrate" => await AdminCommands.MigrateAsync(rest),
    "bootstrap-owner" => await AdminCommands.BootstrapOwnerAsync(rest),
    "broker-bootstrap" => await AdminCommands.BrokerBootstrapAsync(rest),
    "healthcheck" => await AdminCommands.HealthcheckAsync(rest),
    _ => AdminCommands.Usage(command),
};

static async Task<int> RunServerAsync(string[] args)
{
    var app = JarvisApiHost.BuildApp(JarvisApiHost.CreateBuilder(args));
    await app.RunAsync();
    return 0;
}

/// <summary>Punto de entrada visible para WebApplicationFactory en los tests.</summary>
public partial class Program;
