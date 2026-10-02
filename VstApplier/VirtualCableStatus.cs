namespace VstApplier;

public sealed record VirtualCableStatus(
    bool IsReady,
    string OutputDeviceName,
    string InputDeviceName,
    string Message);
