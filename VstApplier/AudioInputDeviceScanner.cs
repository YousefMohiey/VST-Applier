namespace VstApplier;

using NAudio.CoreAudioApi;

public sealed class AudioInputDeviceScanner
{
    public IReadOnlyList<AudioInputDevice> GetInputDevices()
    {
        using var enumerator = new MMDeviceEnumerator();

        return enumerator
            .EnumerateAudioEndPoints(DataFlow.Capture, DeviceState.Active)
            .Select(device => new AudioInputDevice(device.ID, device.FriendlyName))
            .ToList();
    }
}
