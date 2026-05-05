using System.Globalization;
using System.IO;
using System.Text;

namespace HumanizeInput.App.Settings;

public sealed class IniSettingsStore
{
    private readonly string _filePath;

    public IniSettingsStore(string filePath)
    {
        _filePath = filePath;
    }

    public UserSettings LoadOrCreateDefault(UserSettings defaults)
    {
        string? dir = Path.GetDirectoryName(_filePath);
        if (!string.IsNullOrWhiteSpace(dir))
        {
            Directory.CreateDirectory(dir);
        }

        if (!File.Exists(_filePath))
        {
            Save(defaults);
            return Clone(defaults);
        }

        UserSettings merged = Clone(defaults);

        foreach (string rawLine in File.ReadAllLines(_filePath, Encoding.UTF8))
        {
            string line = rawLine.Trim();
            if (line.Length == 0 || line.StartsWith(";") || line.StartsWith("#") || line.StartsWith("["))
            {
                continue;
            }

            int separator = line.IndexOf('=');
            if (separator <= 0)
            {
                continue;
            }

            string key = line[..separator].Trim().ToLowerInvariant();
            string value = line[(separator + 1)..].Trim();
            Apply(merged, key, value);
        }

        // Normalize and append any newly introduced keys.
        Save(merged);
        return merged;
    }

    public void Save(UserSettings settings)
    {
        StringBuilder sb = new();
        sb.AppendLine("[ui]");
        sb.AppendLine($"language={settings.Language}");
        sb.AppendLine();
        sb.AppendLine("[typing]");
        sb.AppendLine($"base_delay_ms={settings.BaseDelayMs}");
        sb.AppendLine($"jitter_percent={settings.JitterPercent}");
        sb.AppendLine($"typo_rate_percent={settings.TypoRatePercent}");
        sb.AppendLine($"omission_rate_percent={settings.OmissionRatePercent}");
        sb.AppendLine($"transpose_rate_percent={settings.TransposeRatePercent}");
        sb.AppendLine($"repair_rate_percent={settings.RepairRatePercent}");
        sb.AppendLine($"error_detect_delay_ms={settings.ErrorDetectDelayMs}");
        sb.AppendLine($"backspace_delay_ms={settings.BackspaceDelayMs}");
        sb.AppendLine($"lead_in_delay_ms={settings.LeadInDelayMs}");
        sb.AppendLine();
        sb.AppendLine("[hotkeys]");
        sb.AppendLine($"start_hotkey={settings.StartHotkeyText}");
        sb.AppendLine($"pause_hotkey={settings.PauseHotkeyText}");

        File.WriteAllText(_filePath, sb.ToString(), Encoding.UTF8);
    }

    private static void Apply(UserSettings settings, string key, string value)
    {
        switch (key)
        {
            case "language":
                settings.Language = string.IsNullOrWhiteSpace(value) ? "en-US" : value;
                break;
            case "base_delay_ms":
                settings.BaseDelayMs = ParseInt(value, settings.BaseDelayMs);
                break;
            case "jitter_percent":
                settings.JitterPercent = ParseInt(value, settings.JitterPercent);
                break;
            case "typo_rate_percent":
                settings.TypoRatePercent = ParseInt(value, settings.TypoRatePercent);
                break;
            case "omission_rate_percent":
                settings.OmissionRatePercent = ParseInt(value, settings.OmissionRatePercent);
                break;
            case "transpose_rate_percent":
                settings.TransposeRatePercent = ParseInt(value, settings.TransposeRatePercent);
                break;
            case "repair_rate_percent":
                settings.RepairRatePercent = ParseInt(value, settings.RepairRatePercent);
                break;
            case "error_detect_delay_ms":
                settings.ErrorDetectDelayMs = ParseInt(value, settings.ErrorDetectDelayMs);
                break;
            case "backspace_delay_ms":
                settings.BackspaceDelayMs = ParseInt(value, settings.BackspaceDelayMs);
                break;
            case "lead_in_delay_ms":
                settings.LeadInDelayMs = ParseInt(value, settings.LeadInDelayMs);
                break;
            case "start_hotkey":
                settings.StartHotkeyText = string.IsNullOrWhiteSpace(value) ? settings.StartHotkeyText : value;
                break;
            case "pause_hotkey":
                settings.PauseHotkeyText = string.IsNullOrWhiteSpace(value) ? settings.PauseHotkeyText : value;
                break;
        }
    }

    private static int ParseInt(string value, int fallback)
    {
        return int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out int parsed)
            ? parsed
            : fallback;
    }

    private static UserSettings Clone(UserSettings source)
    {
        return new UserSettings
        {
            Language = source.Language,
            BaseDelayMs = source.BaseDelayMs,
            JitterPercent = source.JitterPercent,
            TypoRatePercent = source.TypoRatePercent,
            OmissionRatePercent = source.OmissionRatePercent,
            TransposeRatePercent = source.TransposeRatePercent,
            RepairRatePercent = source.RepairRatePercent,
            ErrorDetectDelayMs = source.ErrorDetectDelayMs,
            BackspaceDelayMs = source.BackspaceDelayMs,
            LeadInDelayMs = source.LeadInDelayMs,
            StartHotkeyText = source.StartHotkeyText,
            PauseHotkeyText = source.PauseHotkeyText
        };
    }
}
