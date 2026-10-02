namespace VstApplier;

public sealed record AudioInputDevice(string Id, string Name)
{
    public override string ToString() => Name;
}
