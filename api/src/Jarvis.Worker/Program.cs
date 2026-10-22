using Microsoft.Extensions.Hosting;
using Jarvis.Worker;

// jarvis-worker: jobs programados del Cloud Core (ADR-016). Sin argumentos ejecuta el bucle; "healthcheck" comprueba el latido.
if (args.Length > 0 && args[0] == "healthcheck")
{
    return WorkerHost.Healthcheck(args[1..]);
}

using var host = WorkerHost.CreateBuilder(args).Build();
await host.RunAsync();
return 0;
