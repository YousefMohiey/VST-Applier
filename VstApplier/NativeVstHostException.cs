namespace VstApplier;

public sealed class NativeVstHostException : Exception
{
    public NativeVstHostException(string message)
        : base(message)
    {
    }

    public NativeVstHostException(string message, Exception innerException)
        : base(message, innerException)
    {
    }
}
