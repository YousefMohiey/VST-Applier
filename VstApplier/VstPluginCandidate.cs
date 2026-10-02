namespace VstApplier;

public enum VstPluginFormat
{
    Vst3,
    Vst2,
}

public sealed record VstPluginCandidate(string Path, string Name, VstPluginFormat Format)
{
    public string FormatLabel => Format switch
    {
        VstPluginFormat.Vst2 => "VST2 x64",
        _ => "VST3",
    };

    public override string ToString() => $"{Name} ({FormatLabel})";
}
