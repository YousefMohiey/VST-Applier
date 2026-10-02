namespace VstApplier;

public sealed record AudioRouteDiagnostics(
    double BufferedMs,
    double BufferCapacityMs,
    int RequestedOutputLatencyMs,
    double LastCaptureBlockMs,
    int PluginBlockSize,
    double InitialPreloadMs)
{
    public static AudioRouteDiagnostics Empty { get; } = new(0, 0, 0, 0, 0, 0);
}
