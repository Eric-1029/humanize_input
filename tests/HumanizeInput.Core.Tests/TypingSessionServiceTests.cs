using HumanizeInput.Core;
using HumanizeInput.Core.Models;
using Xunit;

namespace HumanizeInput.Core.Tests;

public class TypingSessionServiceTests
{
    [Fact]
    public async Task StopCanceled_AllowsSessionToRestart()
    {
        FakeTypingDriver driver = new();
        TypingSessionService service = new(driver);
        List<string> logs = [];
        service.LogProduced += logs.Add;

        bool started = service.Start("hello world", FastSettings(), targetWindow: (nint)123);

        Assert.True(started);
        await driver.WaitForFirstCharacterAsync();

        service.Stop(canceled: true);
        await WaitForStateAsync(service, SessionState.Idle);

        Assert.Contains("会话已取消。", logs);
        Assert.False(service.IsBusy);

        bool restarted = service.Start("restart", FastSettings(), targetWindow: (nint)123);

        Assert.True(restarted);

        service.Stop(canceled: true);
        await WaitForStateAsync(service, SessionState.Idle);
    }

    private static TypingSettings FastSettings()
    {
        return new TypingSettings
        {
            BaseDelayMs = 1,
            JitterPercent = 0,
            ErrorDetectDelayMs = 1,
            BackspaceDelayMs = 1
        };
    }

    private static async Task WaitForStateAsync(TypingSessionService service, SessionState expectedState)
    {
        for (int i = 0; i < 50; i++)
        {
            if (service.State == expectedState)
            {
                return;
            }

            await Task.Delay(20);
        }

        Assert.Equal(expectedState, service.State);
    }

    private sealed class FakeTypingDriver : ITypingDriver
    {
        private readonly TaskCompletionSource _firstCharacterTyped = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public nint GetForegroundWindowHandle() => (nint)123;

        public Task TypeCharAsync(char value, CancellationToken cancellationToken)
        {
            _firstCharacterTyped.TrySetResult();
            return Task.Delay(30, cancellationToken);
        }

        public Task BackspaceAsync(CancellationToken cancellationToken)
        {
            return Task.Delay(30, cancellationToken);
        }

        public Task WaitForFirstCharacterAsync() => _firstCharacterTyped.Task;
    }
}
