namespace VstApplier;

public sealed class VirtualCableService
{
    public VirtualCableStatus GetStatus(
        IReadOnlyList<AudioInputDevice> inputDevices,
        IReadOnlyList<AudioOutputDevice> outputDevices)
    {
        var cableInput = outputDevices
            .Select(device => new { Device = device, Score = GetCableInputScore(device) })
            .Where(match => match.Score > 0)
            .OrderByDescending(match => match.Score)
            .Select(match => match.Device)
            .FirstOrDefault();

        var cableOutput = inputDevices
            .Select(device => new { Device = device, Score = GetCableOutputScore(device) })
            .Where(match => match.Score > 0)
            .OrderByDescending(match => match.Score)
            .Select(match => match.Device)
            .FirstOrDefault();

        return (cableInput, cableOutput) switch
        {
            ({ } renderDevice, { } captureDevice) => new VirtualCableStatus(
                true,
                renderDevice.Name,
                captureDevice.Name,
                "Virtual cable ready"),
            ({ } renderDevice, null) => new VirtualCableStatus(
                false,
                renderDevice.Name,
                "CABLE Output missing",
                "Virtual cable incomplete"),
            (null, { } captureDevice) => new VirtualCableStatus(
                false,
                "CABLE Input missing",
                captureDevice.Name,
                "Virtual cable incomplete"),
            _ => new VirtualCableStatus(
                false,
                "VB-CABLE",
                "Not detected",
                "Install VB-CABLE"),
        };
    }

    public AudioOutputDevice? FindPreferredOutputDevice(IReadOnlyList<AudioOutputDevice> outputDevices)
    {
        return outputDevices
            .Select(device => new { Device = device, Score = GetCableInputScore(device) })
            .Where(match => match.Score > 0)
            .OrderByDescending(match => match.Score)
            .Select(match => match.Device)
            .FirstOrDefault();
    }

    private static int GetCableInputScore(AudioOutputDevice device)
    {
        if (device.Name.Equals("CABLE Input", StringComparison.OrdinalIgnoreCase))
        {
            return 100;
        }

        if (device.Name.Contains("CABLE Input", StringComparison.OrdinalIgnoreCase) &&
            !device.Name.Contains("16ch", StringComparison.OrdinalIgnoreCase))
        {
            return 90;
        }

        if (device.Name.Contains("CABLE Input", StringComparison.OrdinalIgnoreCase))
        {
            return 70;
        }

        if (device.Name.Contains("VB-Audio Virtual Cable", StringComparison.OrdinalIgnoreCase) &&
            !device.Name.Contains("16ch", StringComparison.OrdinalIgnoreCase))
        {
            return 40;
        }

        return device.Name.Contains("VB-Audio Virtual Cable", StringComparison.OrdinalIgnoreCase) ? 20 : 0;
    }

    private static int GetCableOutputScore(AudioInputDevice device)
    {
        if (device.Name.Equals("CABLE Output", StringComparison.OrdinalIgnoreCase))
        {
            return 100;
        }

        if (device.Name.Contains("CABLE Output", StringComparison.OrdinalIgnoreCase))
        {
            return 90;
        }

        return device.Name.Contains("VB-Audio Virtual Cable", StringComparison.OrdinalIgnoreCase) ? 40 : 0;
    }
}
