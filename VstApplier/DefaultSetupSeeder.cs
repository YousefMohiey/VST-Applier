namespace VstApplier;

/// <summary>
/// Installs the bundled default setup (settings, default profile and session) for
/// first-run users, so the app opens with the project's plugins and profile ready.
/// The bundled files are templates: paths contain an {APP} placeholder that is
/// replaced with this installation's folder at seed time. Existing user data is
/// never touched: seeding only happens when there is no settings file and no profile.
/// </summary>
public static class DefaultSetupSeeder
{
    private const string AppPlaceholder = "{APP}";

    public static void EnsureSeeded()
    {
        try
        {
            var defaultsDirectory = Path.Combine(AppContext.BaseDirectory, "defaults");
            if (!Directory.Exists(defaultsDirectory))
            {
                return;
            }

            var settingsPath = AppSettingsStore.SettingsFilePath;
            var profilesDirectory = VoiceSetupStore.ProfilesDirectory;
            var hasProfiles = Directory.Exists(profilesDirectory) &&
                Directory.EnumerateFiles(profilesDirectory, "*.json").Any();
            if (File.Exists(settingsPath) || hasProfiles)
            {
                return;
            }

            SeedFrom(
                defaultsDirectory,
                VoiceSetupStore.SettingsDirectory,
                AppContext.BaseDirectory.TrimEnd(
                    Path.DirectorySeparatorChar,
                    Path.AltDirectorySeparatorChar));
        }
        catch
        {
            // Defaults are best effort; never block startup because of them.
        }
    }

    /// <summary>
    /// Copies the template files into the target directory, replacing {APP} with the
    /// app directory. The placeholder always sits inside JSON string values, so
    /// backslashes in the replacement are escaped for JSON.
    /// </summary>
    internal static void SeedFrom(string defaultsDirectory, string targetDirectory, string appDirectory)
    {
        var jsonAppDirectory = appDirectory.Replace("\\", "\\\\");

        foreach (var sourcePath in Directory.EnumerateFiles(defaultsDirectory, "*", SearchOption.AllDirectories))
        {
            var relativePath = Path.GetRelativePath(defaultsDirectory, sourcePath);
            var targetPath = Path.Combine(targetDirectory, relativePath);
            Directory.CreateDirectory(Path.GetDirectoryName(targetPath)!);

            var text = File.ReadAllText(sourcePath)
                .Replace(AppPlaceholder, jsonAppDirectory, StringComparison.Ordinal);
            File.WriteAllText(targetPath, text);
        }
    }
}
