using System.Text.Json;

namespace VstApplier;

public static class VoiceSetupStore
{
    private const int MaxProfileNameLength = 48;

    private static readonly JsonSerializerOptions SerializerOptions = new()
    {
        WriteIndented = true,
    };

    static VoiceSetupStore()
    {
        MigrateLegacySettingsFolder();
    }

    public static string SettingsDirectory { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "VstApplier");

    public static string SessionFilePath { get; } = Path.Combine(SettingsDirectory, "session.json");

    public static string ProfilesDirectory { get; } = Path.Combine(SettingsDirectory, "profiles");

    private static void MigrateLegacySettingsFolder()
    {
        try
        {
            // Folder name used by earlier versions of the app. Checked once at startup so
            // existing profiles and settings carry over; the folder itself is not deleted.
            var legacyDirectory = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                "SnjVoiceChanger");

            if (!Directory.Exists(legacyDirectory) || Directory.Exists(SettingsDirectory))
            {
                return;
            }

            CopyDirectory(legacyDirectory, SettingsDirectory);
        }
        catch
        {
            // Migration is best effort; a failed copy must not break startup.
        }
    }

    private static void CopyDirectory(string sourceDirectory, string destinationDirectory)
    {
        Directory.CreateDirectory(destinationDirectory);

        foreach (var filePath in Directory.EnumerateFiles(sourceDirectory))
        {
            File.Copy(filePath, Path.Combine(destinationDirectory, Path.GetFileName(filePath)), overwrite: false);
        }

        foreach (var directoryPath in Directory.EnumerateDirectories(sourceDirectory))
        {
            CopyDirectory(directoryPath, Path.Combine(destinationDirectory, Path.GetFileName(directoryPath)));
        }
    }

    public static VoiceSetupProfile? LoadSession()
    {
        return TryReadProfile(SessionFilePath);
    }

    public static void SaveSession(VoiceSetupProfile profile)
    {
        Directory.CreateDirectory(SettingsDirectory);
        File.WriteAllText(SessionFilePath, JsonSerializer.Serialize(profile, SerializerOptions));
    }

    public static IReadOnlyList<string> GetProfileNames()
    {
        if (!Directory.Exists(ProfilesDirectory))
        {
            return [];
        }

        return Directory
            .EnumerateFiles(ProfilesDirectory, "*.json")
            .Select(Path.GetFileNameWithoutExtension)
            .Where(name => !string.IsNullOrWhiteSpace(name))
            .Select(name => name!)
            .OrderBy(name => name, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    public static VoiceSetupProfile? LoadProfile(string name)
    {
        var path = GetProfileFilePath(name);
        return path is null ? null : TryReadProfile(path);
    }

    public static void SaveProfile(VoiceSetupProfile profile)
    {
        if (!IsValidProfileName(profile.Name, out var error))
        {
            throw new InvalidOperationException(error);
        }

        var path = GetProfileFilePath(profile.Name)!;
        Directory.CreateDirectory(ProfilesDirectory);
        File.WriteAllText(path, JsonSerializer.Serialize(profile, SerializerOptions));
    }

    public static void DeleteProfile(string name)
    {
        var path = GetProfileFilePath(name);
        if (path is not null && File.Exists(path))
        {
            File.Delete(path);
        }
    }

    public static bool ProfileExists(string name)
    {
        var path = GetProfileFilePath(name);
        return path is not null && File.Exists(path);
    }

    public static bool IsValidProfileName(string? name, out string error)
    {
        error = string.Empty;
        var trimmed = name?.Trim() ?? string.Empty;

        if (trimmed.Length == 0)
        {
            error = "Enter a profile name.";
            return false;
        }

        if (trimmed.Length > MaxProfileNameLength)
        {
            error = $"Profile name must be {MaxProfileNameLength} characters or fewer.";
            return false;
        }

        if (trimmed is "." or ".." ||
            trimmed.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0 ||
            trimmed.Contains('/') ||
            trimmed.Contains('\\'))
        {
            error = "Profile name contains invalid characters.";
            return false;
        }

        return true;
    }

    private static string? GetProfileFilePath(string name)
    {
        if (!IsValidProfileName(name, out _))
        {
            return null;
        }

        return Path.Combine(ProfilesDirectory, name.Trim() + ".json");
    }

    private static VoiceSetupProfile? TryReadProfile(string path)
    {
        try
        {
            if (!File.Exists(path))
            {
                return null;
            }

            return JsonSerializer.Deserialize<VoiceSetupProfile>(File.ReadAllText(path));
        }
        catch
        {
            return null;
        }
    }
}
