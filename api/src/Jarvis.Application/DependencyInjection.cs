using Jarvis.Application.Abstractions;
using Jarvis.Application.Auth;
using Jarvis.Application.Common;
using Jarvis.Application.Configuration;
using Jarvis.Application.Devices;
using Jarvis.Application.Events;
using Jarvis.Application.Mqtt;
using Jarvis.Application.Presence;
using Jarvis.Application.Realtime;
using Jarvis.Application.Security;
using Jarvis.Application.Tasks;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;

namespace Jarvis.Application;

public static class DependencyInjection
{
    public static IServiceCollection AddJarvisApplication(this IServiceCollection services, IConfiguration configuration)
    {
        services.AddOptions<JarvisOptions>().Bind(configuration.GetSection(JarvisOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<MqttOptions>().Bind(configuration.GetSection(MqttOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<JwtOptions>().Bind(configuration.GetSection(JwtOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<TaskOptions>().Bind(configuration.GetSection(TaskOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<PresenceOptions>().Bind(configuration.GetSection(PresenceOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<SecurityOptions>().Bind(configuration.GetSection(SecurityOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<RetentionOptions>().Bind(configuration.GetSection(RetentionOptions.Section)).ValidateDataAnnotations().ValidateOnStart();
        services.AddOptions<BrokerReconciliationOptions>().Bind(configuration.GetSection(BrokerReconciliationOptions.Section)).ValidateDataAnnotations().ValidateOnStart();

        services.TryAddSingleton(TimeProvider.System);
        services.AddMemoryCache();
        services.AddMetrics();

        services.AddScoped<IRequestContext, RequestContext>();
        services.AddSingleton<PostCommitEventChannel>();
        services.AddSingleton<IPostCommitEventSink>(sp => sp.GetRequiredService<PostCommitEventChannel>());
        services.AddSingleton<DispatchSignal>();
        services.AddSingleton<ServiceHeartbeats>();
        services.AddSingleton<JarvisMetrics>();
        services.AddSingleton<PrincipalStatusService>();
        services.AddSingleton<DeviceRateLimits>();
        services.AddSingleton<MqttInboundProcessor>();
        services.AddSingleton<BrokerReconciler>();

        services.AddScoped<BootstrapOwnerHandler>();
        services.AddScoped<ResetOwnerPasswordHandler>();
        services.AddScoped<LoginHandler>();
        services.AddScoped<RefreshHandler>();
        services.AddScoped<LogoutHandler>();
        services.AddScoped<DeviceTokenHandler>();
        services.AddScoped<MeHandler>();
        services.AddScoped<RegisterDeviceHandler>();
        services.AddScoped<DeviceQueryHandler>();
        services.AddScoped<DeviceAdminHandler>();
        services.AddScoped<GrantHandler>();
        services.AddScoped<CreateTaskHandler>();
        services.AddScoped<TaskQueryHandler>();
        services.AddScoped<CancelTaskHandler>();
        services.AddScoped<DecideTaskApprovalHandler>();
        services.AddScoped<TaskDispatchService>();
        services.AddScoped<TaskSweepService>();
        services.AddScoped<DeviceTaskUpdateHandler>();
        services.AddScoped<DeviceRequestHandler>();
        services.AddScoped<PresenceHandler>();
        services.AddScoped<PresenceSweepService>();
        services.AddScoped<EventQueryHandler>();
        services.AddScoped<RetentionService>();

        // Memoria, sincronización, habilidades y jobs (ADR-013 … ADR-016).
        services.AddScoped<Sync.SyncAccess>();
        services.AddScoped<Sync.SyncPushHandler>();
        services.AddScoped<Sync.SyncPullHandler>();
        services.AddScoped<Sync.SyncDeviceHandler>();
        services.AddScoped<Sync.SyncConflictHandler>();
        services.AddScoped<Sync.MemoryQueryHandler>();
        services.AddScoped<Sync.ExperienceQueryHandler>();
        services.AddScoped<Sync.SkillHandler>();
        services.AddScoped<Jobs.JobAdminHandler>();

        services.AddScoped<IEventSubscriber, RealtimeEventSubscriber>();
        services.AddScoped<IEventSubscriber, SourceDeviceTaskNotifier>();
        services.AddScoped<IEventSubscriber, MetricsEventSubscriber>();
        return services;
    }

    /// <summary>Planificador y handlers de jobs que solo usan la base de datos (los registra el worker).</summary>
    public static IServiceCollection AddJarvisJobs(this IServiceCollection services)
    {
        services.TryAddSingleton(new Jobs.JobSchedulerOptions());
        services.AddScoped<Jobs.JobScheduler>();
        services.AddScoped<Jobs.IJobHandler, Jobs.ReportSummaryJob>();
        services.AddScoped<Jobs.IJobHandler, Jobs.MemoryConsolidateJob>();
        services.AddScoped<Jobs.IJobHandler, Jobs.SyncCompactJob>();
        services.AddScoped<Jobs.IJobHandler, Jobs.MemoryPurgeExpiredJob>();
        return services;
    }
}
