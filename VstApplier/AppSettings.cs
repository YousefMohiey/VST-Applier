using System.Text.Json;

namespace VstApplier;

public sealed class AppSettings
{
    public bool AutoStartMicrophone { get; set; } = true;

    public bool CloseToTray { get; set; } = true;

    public bool StartWithWindows { get; set; }
}

public static class AppSettingsStore
{
    private static readonly JsonSerializerOptions SerializerOptions = new()
    {
        WriteIndented = true,
    };

    public static string SettingsFilePath { get; } = Path.Combine(
        VoiceSetupStore.SettingsDirectory,
        "settings.json");

    public static AppSettings Load()
    {
        try
        {
            if (!File.Exists(SettingsFilePath))
            {
                return new AppSettings();
            }

            return JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(SettingsFilePath))
                ?? new AppSettings();
        }
        catch
        {
            return new AppSettings();
        }
    }

    public static void Save(AppSettings settings)
    {
        Directory.CreateDirectory(VoiceSetupStore.SettingsDirectory);
        File.WriteAllText(SettingsFilePath, JsonSerializer.Serialize(settings, SerializerOptions));
    }
}
