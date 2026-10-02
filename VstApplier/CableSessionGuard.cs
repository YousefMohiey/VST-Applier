using NAudio.CoreAudioApi;

namespace VstApplier;

/// <summary>
/// Keeps the virtual cable clean: while enabled, any other application that plays
/// audio to the cable device is muted, so only VST-Applier's processed voice travels
/// through it. This is the same as muting the app in the Windows volume mixer, done
/// automatically and restored when the app exits or the option is turned off.
/// The guard only acts when the selected output device looks like a virtual cable,
/// so real playback devices are never touched. Sessions that were already muted by
/// the user are left alone.
/// </summary>
public sealed class CableSessionGuard : IDisposable
{
    private const int PollIntervalMilliseconds = 2000;

    private readonly Func<AudioOutputDevice?> _outputDeviceProvider;
    private readonly object _sync = new();
    private readonly List<MutedSession> _mutedSessions = new();
    private readonly ManualResetEvent _stopped = new(false);
    private readonly Thread _worker;
    private volatile bool _enabled;
    private bool _disposed;

    private sealed record MutedSession(string InstanceId, SimpleAudioVolume Volume);

    public CableSessionGuard(Func<AudioOutputDevice?> outputDeviceProvider, bool enabled)
    {
        _outputDeviceProvider = outputDeviceProvider;
        _enabled = enabled;
        _worker = new Thread(WorkerLoop)
        {
            IsBackground = true,
            Name = "CableSessionGuard",
        };
        _worker.Start();
    }

    public bool Enabled
    {
        get => _enabled;
        set
        {
            _enabled = value;
            if (!value)
            {
                RestoreMutedSessions();
            }
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        _enabled = false;
        _stopped.Set();
        _worker.Join(2000);
        RestoreMutedSessions();
    }

    private void WorkerLoop()
    {
        while (true)
        {
            try
            {
                GuardOnce();
            }
            catch
            {
                // Audio sessions can fail while devices change; try again next tick.
            }

            if (_stopped.WaitOne(PollIntervalMilliseconds))
            {
                return;
            }
        }
    }

    private void GuardOnce()
    {
        if (!_enabled)
        {
            RestoreMutedSessions();
            return;
        }

        var outputDevice = _outputDeviceProvider();
        if (outputDevice is null || !LooksLikeVirtualCable(outputDevice.Name))
        {
            RestoreMutedSessions();
            return;
        }

        using var enumerator = new MMDeviceEnumerator();
        MMDevice device;
        try
        {
            device = enumerator.GetDevice(outputDevice.Id);
        }
        catch
        {
            RestoreMutedSessions();
            return;
        }

        if (device.State != DeviceState.Active)
        {
            RestoreMutedSessions();
            return;
        }

        var sessions = device.AudioSessionManager.Sessions;
        var ownProcessId = Environment.ProcessId;

        for (var i = 0; i < sessions.Count; i++)
        {
            using var session = sessions[i];
            if (session.GetProcessID == ownProcessId)
            {
                continue;
            }

            string instanceId;
            SimpleAudioVolume volume;
            try
            {
                instanceId = session.GetSessionInstanceIdentifier;
                volume = session.SimpleAudioVolume;
            }
            catch
            {
                continue;
            }

            MuteSession(instanceId, volume);
        }
    }

    private void MuteSession(string instanceId, SimpleAudioVolume volume)
    {
        bool alreadyMuted;
        try
        {
            alreadyMuted = volume.Mute;
        }
        catch
        {
            return;
        }

        if (alreadyMuted)
        {
            // The user muted this app themselves; leave it alone.
            return;
        }

        try
        {
            volume.Mute = true;
        }
        catch
        {
            return;
        }

        lock (_sync)
        {
            if (!_mutedSessions.Any(session => session.InstanceId == instanceId))
            {
                _mutedSessions.Add(new MutedSession(instanceId, volume));
            }
        }
    }

    private void RestoreMutedSessions()
    {
        List<MutedSession> toRestore;
        lock (_sync)
        {
            if (_mutedSessions.Count == 0)
            {
                return;
            }

            toRestore = new List<MutedSession>(_mutedSessions);
            _mutedSessions.Clear();
        }

        foreach (var session in toRestore)
        {
            try
            {
                session.Volume.Mute = false;
            }
            catch
            {
                // The app's audio session already ended; nothing to restore.
            }
        }
    }

    private static bool LooksLikeVirtualCable(string deviceName) =>
        deviceName.Contains("cable", StringComparison.OrdinalIgnoreCase) ||
        deviceName.Contains("virtual", StringComparison.OrdinalIgnoreCase) ||
        deviceName.Contains("voicemeeter", StringComparison.OrdinalIgnoreCase);
}
