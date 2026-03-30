namespace HumanizeInput.App.Settings;

public sealed class UserSettings
{
    public int BaseDelayMs { get; set; } = 90;
    public int JitterPercent { get; set; } = 20;
    public int TypoRatePercent { get; set; } = 8;
    public int OmissionRatePercent { get; set; } = 5;
    public int TransposeRatePercent { get; set; } = 4;
    public int RepairRatePercent { get; set; } = 85;
    public int ErrorDetectDelayMs { get; set; } = 900;
    public int BackspaceDelayMs { get; set; } = 70;
    public int LeadInDelayMs { get; set; } = 2500;
    public string StartHotkeyText { get; set; } = "Ctrl+Alt+S";
    public string PauseHotkeyText { get; set; } = "Ctrl+Alt+P";
}
