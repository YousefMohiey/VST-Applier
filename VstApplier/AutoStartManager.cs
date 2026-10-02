using Microsoft.Win32;

namespace VstApplier;

internal static class AutoStartManager
{
    private const string RunKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Run";

    private const string ValueName = "VST-Applier";

    public static void SetEnabled(bool enabled)
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKeyPath, writable: true)
            ?? Registry.CurrentUser.CreateSubKey(RunKeyPath);

        if (key is null)
        {
            throw new InvalidOperationException("Could not open the Windows startup registry key.");
        }

        if (enabled)
        {
            var executablePath = Application.ExecutablePath;
            key.SetValue(ValueName, $"\"{executablePath}\" --tray");
        }
        else
        {
            key.DeleteValue(ValueName, throwOnMissingValue: false);
        }
    }
}
