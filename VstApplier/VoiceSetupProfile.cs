namespace VstApplier;

public sealed class VoiceSetupProfile
{
    public int SchemaVersion { get; set; } = 1;

    public string Name { get; set; } = string.Empty;

    public string? ActiveProfile { get; set; }

    public string? PluginFolder { get; set; }

    public string? InputDeviceId { get; set; }

    public string? InputDeviceName { get; set; }

    public string? OutputDeviceId { get; set; }

    public string? OutputDeviceName { get; set; }

    public int BufferSize { get; set; } = 512;

    public List<VoiceSetupPluginEntry> Plugins { get; set; } = [];
}

public sealed class VoiceSetupPluginEntry
{
    public string Name { get; set; } = string.Empty;

    public string Path { get; set; } = string.Empty;

    public string Format { get; set; } = "Vst3";

    public bool Enabled { get; set; } = true;

    public string? State { get; set; }
}
