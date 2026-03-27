using HumanizeInput.Core.Timing;
using Xunit;

namespace HumanizeInput.Core.Tests;

public class DelayModelTests
{
    [Fact]
    public void NextDelayMs_ShouldStayInConfiguredRange()
    {
        Random random = new(42);
        int baseDelay = 100;
        int jitter = 20;

        for (int i = 0; i < 500; i++)
        {
            int value = DelayModel.NextDelayMs(random, baseDelay, jitter);
            Assert.InRange(value, 80, 120);
        }
    }
}
