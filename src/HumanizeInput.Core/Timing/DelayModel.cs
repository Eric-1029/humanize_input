namespace HumanizeInput.Core.Timing;

public static class DelayModel
{
    public static int NextDelayMs(Random random, int baseDelayMs, int jitterPercent)
    {
        int safeBase = Math.Max(baseDelayMs, 1);
        int safeJitter = Math.Clamp(jitterPercent, 0, 100);
        double jitterFactor = safeJitter / 100.0;
        double span = safeBase * jitterFactor;
        double min = safeBase - span;
        double max = safeBase + span;
        return (int)Math.Round(min + random.NextDouble() * (max - min));
    }
}
