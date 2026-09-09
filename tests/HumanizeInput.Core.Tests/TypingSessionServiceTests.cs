using HumanizeInput.Core;
using HumanizeInput.Core.Models;
using Xunit;

namespace HumanizeInput.Core.Tests;

public class TypingSessionServiceTests
{
    [Fact]
    public async Task Start_NormalizesWindowsAndLegacyNewlines()
    {
        RecordingTypingDriver driver = new();
        TypingSessionService service = new(driver);

        bool started = service.Start("a\r\nb\rc\nd", FastSettings(), targetWindow: (nint)123);

        Assert.True(started);
        await WaitForStateAsync(service, SessionState.Completed);
        Assert.Equal("a\nb\nc\nd", driver.TypedText);
    }

    [Fact]
    public async Task Stop_RejectsRestartUntilCanceledSessionHasExited()
    {
        BlockingTypingDriver driver = new();
        TypingSessionService service = new(driver);

        Assert.True(service.Start("first", FastSettings(), targetWindow: (nint)123));
        await driver.WaitUntilBlockedAsync();

        service.Stop(canceled: true);

        Assert.False(service.Start("too soon", FastSettings(), targetWindow: (nint)123));

        driver.Release();
        await WaitForStateAsync(service, SessionState.Idle);

        Assert.True(service.Start("restart", FastSettings(), targetWindow: (nint)123));
        await WaitForStateAsync(service, SessionState.Completed);
    }

    [Fact]
    public async Task Correction_WaitsForTargetFocusBeforeBackspacing()
    {
        FocusLosingTypingDriver driver = new();
        TypingSessionService service = new(driver);
        TypingSettings settings = new()
        {
            BaseDelayMs = 1,
            JitterPercent = 0,
            TypoRatePercent = 100,
            OmissionRatePercent = 0,
            TransposeRatePercent = 0,
            RepairRatePercent = 100,
            ErrorDetectDelayMs = 1,
            BackspaceDelayMs = 1
        };

        Assert.True(service.Start("a", settings, targetWindow: (nint)123));
        await driver.WaitForFirstCharacterAsync();
        await WaitForStateAsync(service, SessionState.Paused);

        Assert.False(driver.HasBackspaced);

        driver.RestoreTargetFocus();
        await driver.WaitForBackspaceAsync();
        await WaitForStateAsync(service, SessionState.Completed);
    }

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
            TypoRatePercent = 0,
            OmissionRatePercent = 0,
            TransposeRatePercent = 0,
            RepairRatePercent = 0,
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

    private sealed class RecordingTypingDriver : ITypingDriver
    {
        private readonly List<char> _typedCharacters = [];

        public string TypedText
        {
            get
            {
                lock (_typedCharacters)
                {
                    return new string([.. _typedCharacters]);
                }
            }
        }

        public nint GetForegroundWindowHandle() => (nint)123;

        public Task TypeCharAsync(char value, CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            lock (_typedCharacters)
            {
                _typedCharacters.Add(value);
            }

            return Task.CompletedTask;
        }

        public Task BackspaceAsync(CancellationToken cancellationToken)
        {
            throw new InvalidOperationException("Backspace was not expected.");
        }
    }

    private sealed class BlockingTypingDriver : ITypingDriver
    {
        private readonly TaskCompletionSource _blocked = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource _release = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public nint GetForegroundWindowHandle() => (nint)123;

        public async Task TypeCharAsync(char value, CancellationToken cancellationToken)
        {
            _blocked.TrySetResult();
            await _release.Task;
        }

        public Task BackspaceAsync(CancellationToken cancellationToken)
        {
            throw new InvalidOperationException("Backspace was not expected.");
        }

        public Task WaitUntilBlockedAsync() => _blocked.Task;

        public void Release() => _release.TrySetResult();
    }

    private sealed class FocusLosingTypingDriver : ITypingDriver
    {
        private readonly TaskCompletionSource _firstCharacterTyped = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource _backspaced = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private long _foregroundWindow = 123;
        private int _typedCharacterCount;

        public bool HasBackspaced => _backspaced.Task.IsCompleted;

        public nint GetForegroundWindowHandle() => (nint)Interlocked.Read(ref _foregroundWindow);

        public Task TypeCharAsync(char value, CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            if (Interlocked.Increment(ref _typedCharacterCount) == 1)
            {
                Interlocked.Exchange(ref _foregroundWindow, 456);
                _firstCharacterTyped.TrySetResult();
            }

            return Task.CompletedTask;
        }

        public Task BackspaceAsync(CancellationToken cancellationToken)
        {
            cancellationToken.ThrowIfCancellationRequested();
            _backspaced.TrySetResult();
            return Task.CompletedTask;
        }

        public Task WaitForFirstCharacterAsync() => _firstCharacterTyped.Task;

        public Task WaitForBackspaceAsync() => _backspaced.Task;

        public void RestoreTargetFocus() => Interlocked.Exchange(ref _foregroundWindow, 123);
    }
}
