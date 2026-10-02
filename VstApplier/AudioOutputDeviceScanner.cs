namespace VstApplier;

using NAudio.CoreAudioApi;

public sealed class AudioOutputDeviceScanner
{
    public IReadOnlyList<AudioOutputDevice> GetOutputDevices()
    {
        using var enumerator = new MMDeviceEnumerator();

        return enumerator
            .EnumerateAudioEndPoints(DataFlow.Render, DeviceState.Active)
            .Select(device => new AudioOutputDevice(device.ID, device.FriendlyName))
            .ToList();
    }
}
