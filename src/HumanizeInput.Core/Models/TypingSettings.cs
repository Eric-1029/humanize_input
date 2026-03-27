namespace HumanizeInput.Core.Models;

public sealed class TypingSettings
{
    public int BaseDelayMs { get; init; } = 90;
    public int JitterPercent { get; init; } = 20;
    public int TypoRatePercent { get; init; } = 8;
    public int OmissionRatePercent { get; init; } = 5;
    public int TransposeRatePercent { get; init; } = 4;
    public int RepairRatePercent { get; init; } = 85;
    public int ErrorDetectDelayMs { get; init; } = 900;
    public int BackspaceDelayMs { get; init; } = 70;
    public int LeadInDelayMs { get; init; } = 2500;
}
