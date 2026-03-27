namespace HumanizeInput.Core.Models;

public sealed class TypingProgress
{
    public int TotalChars { get; init; }
    public int TypedChars { get; init; }
    public int TypoCount { get; init; }
    public int OmissionCount { get; init; }
    public int TransposeCount { get; init; }
    public int CorrectionCount { get; init; }
}
